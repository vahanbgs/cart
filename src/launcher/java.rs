use std::{
    collections::HashMap,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use tempfile::TempDir;
use tokio::{
    fs::{self, File},
    process::Command,
};
use url::Url;
use zip::ZipArchive;

use crate::api::{
    forge::MavenCoordinate,
    piston::{
        Action, FileSystemEntry, JavaDistribution, JavaDistributionManifest, JavaPlatform,
        JavaVersionComponent, LibraryEntry, NativeClassifier, Os, Version,
    },
};

static MOJANG_LIBRARIES_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://libraries.minecraft.net/").unwrap());

static FORGE_MAVEN_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://maven.minecraftforge.net/").unwrap());

use super::cache::Cache;

async fn make_executable(path: impl AsRef<Path>) -> anyhow::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o755)).await?;

    Ok(())
}

async fn fetch_java_distribution_manifest(
    cache: &Cache,
) -> anyhow::Result<JavaDistributionManifest> {
    use crate::api::Endpoint;
    cache
        .fetch_json(JavaDistributionManifest::url(), None)
        .await
}

pub async fn fetch_java_distribution(
    java_version_component: JavaVersionComponent,
    cache: &Cache,
) -> anyhow::Result<PathBuf> {
    let java_distribution_manifest = fetch_java_distribution_manifest(cache).await?;

    let java_distribution_info =
        &java_distribution_manifest.0[&JavaPlatform::CURRENT][&java_version_component][0];

    let java_distribution = cache
        .fetch_json::<JavaDistribution>(
            &java_distribution_info.manifest.url,
            Some(&java_distribution_info.manifest.sha1),
        )
        .await?;

    let java_distribution_path = cache
        .directory()
        .join("java")
        .join(java_version_component.as_ref());

    for (path, fs_entry) in java_distribution.files {
        match fs_entry {
            FileSystemEntry::File {
                downloads,
                executable,
            } => {
                let source_path = cache
                    .fetch(&downloads.raw.url, Some(&downloads.raw.sha1))
                    .await?;

                let target_path = java_distribution_path.join(path);

                fs::create_dir_all(target_path.parent().unwrap()).await?;

                if !fs::try_exists(&target_path).await? {
                    fs::hard_link(source_path, &target_path).await?;
                }

                if executable {
                    make_executable(target_path).await?;
                }
            }
            _ => {}
        }
    }

    Ok(java_distribution_path)
}

/// Builds the `-cp` classpath string.
///
/// `client_jar` is the first entry — pass the vanilla client JAR for a plain
/// launch or the Forge-patched JAR when Forge is active. `extra_libraries` are
/// merged after the vanilla libraries; entries with the same
/// `groupId:artifactId[:classifier]` key REPLACE the earlier one rather than
/// appending, matching the Mojang launcher's `inheritsFrom` semantics.
///
/// Without dedup, Forge for 1.16.5 (ships log4j 2.15.0) would end up with
/// vanilla's log4j 2.8.1 also on the classpath and loaded first, so
/// modlauncher (compiled against 2.15.0's API) crashes with
/// `NoSuchMethodError` on ThrowablePatternConverter.
pub async fn build_class_path(
    version_manifest: &Version,
    client_jar: &Path,
    extra_libraries: &[LibraryEntry],
    natives_directory: &TempDir,
    cache: &Cache,
) -> anyhow::Result<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut key_positions: HashMap<String, usize> = HashMap::new();

    // Client jar goes first with a synthetic key that can't collide with
    // any real Maven coordinate.
    add_classpath_entry(
        &mut entries,
        &mut key_positions,
        "__client_jar__".to_string(),
        client_jar.to_string_lossy().into_owned(),
    );

    for library_entry in &version_manifest.libraries {
        let mut allow = library_entry.rules.is_none();

        if let Some(rules) = &library_entry.rules {
            allow = rules.is_empty();

            for rule in rules {
                let mut rule_applies = true;

                if let Some(os) = &rule.os {
                    rule_applies &= match os {
                        Os::Arch { arch: _ } => true,
                        Os::Name { name } => name.matches_current_platform(),
                    }
                }

                rule_applies &= matches!(rule.features, None);

                if rule_applies {
                    allow = rule.action == Action::Allow;
                }
            }
        }

        if !allow {
            continue;
        }

        if let Some(artifact) = &library_entry.downloads.artifact {
            if let Some(url) = &artifact.url {
                let path = cache.fetch(url, Some(&artifact.sha1)).await?;
                add_classpath_entry(
                    &mut entries,
                    &mut key_positions,
                    dedup_key(&library_entry.name),
                    path.to_string_lossy().into_owned(),
                );
            }
        }

        if let Some(native) = &library_entry.downloads.classifiers {
            if let Some(native) = native.get(&NativeClassifier::current()) {
                if let Some(url) = &native.url {
                    let jar_path = cache.fetch(url, Some(&native.sha1)).await?;
                    let jar_file = File::open(jar_path).await?;
                    let mut archive = ZipArchive::new(jar_file.into_std().await)?;

                    archive.extract(natives_directory)?;
                }
            }
        }
    }

    for lib in extra_libraries {
        if lib.clientreq == Some(false) {
            continue;
        }

        let path = if let Some(url) = lib.downloads.artifact.as_ref().and_then(|a| a.url.as_ref()) {
            // Modern format: explicit download URL.
            let sha1 = lib.downloads.artifact.as_ref().map(|a| &a.sha1);
            cache.fetch(url, sha1).await?
        } else if let Some(artifact) = &lib.downloads.artifact {
            // downloads.artifact present but URL is empty: the JAR is bundled
            // in the Forge installer. forge::install() already extracted it to
            // the Forge Maven cache path derived from artifact.path.
            let url = FORGE_MAVEN_URL
                .join(&artifact.path.to_string_lossy())
                .with_context(|| format!("failed to build Forge Maven URL for: {}", lib.name))?;
            cache.fetch(&url, Some(&artifact.sha1)).await?
        } else {
            // Legacy format (no downloads field): build URL from lib.url base
            // or the default Mojang libraries host.
            let coord = MavenCoordinate::parse(&lib.name)
                .with_context(|| format!("failed to parse Maven coordinate: {}", lib.name))?;
            let base = lib.url.as_ref().unwrap_or(&MOJANG_LIBRARIES_URL);
            let url = base
                .join(&coord.to_path().to_string_lossy())
                .with_context(|| format!("failed to build URL for library: {}", lib.name))?;
            cache.fetch(&url, None).await?
        };

        add_classpath_entry(
            &mut entries,
            &mut key_positions,
            dedup_key(&lib.name),
            path.to_string_lossy().into_owned(),
        );
    }

    let paths: Vec<String> = entries.into_iter().map(|(_, path)| path).collect();
    Ok(paths.join(":"))
}

/// Maven coordinate → the key we dedup by: `groupId:artifactId[:classifier]`,
/// dropping the version. Different-classifier entries stay separate (natives
/// jars must coexist with their base artifact), same-classifier entries at
/// different versions collapse to a single classpath slot.
fn dedup_key(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    match parts.as_slice() {
        [group, artifact, _version] => format!("{group}:{artifact}"),
        [group, artifact, _version, classifier, ..] => {
            format!("{group}:{artifact}:{classifier}")
        }
        _ => name.to_owned(),
    }
}

fn add_classpath_entry(
    entries: &mut Vec<(String, String)>,
    key_positions: &mut HashMap<String, usize>,
    key: String,
    path: String,
) {
    if let Some(&idx) = key_positions.get(&key) {
        entries[idx].1 = path;
    } else {
        key_positions.insert(key.clone(), entries.len());
        entries.push((key, path));
    }
}

pub fn java_binary(java_path: &Path) -> Command {
    Command::new(java_path.join("bin").join("java"))
}
