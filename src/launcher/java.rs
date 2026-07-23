use std::{
    collections::HashMap,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::Context;
use futures::{StreamExt, TryStreamExt};
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

/// Max in-flight Java-runtime file fetches. A JRE distribution ships
/// ~50 raw files (binaries, jmods, libs); the same RTT bottleneck as
/// the asset store, at smaller N. Matches the asset-loop budget so we
/// don't need to reason about two independent per-host caps.
const JAVA_FETCH_CONCURRENCY: usize = 8;

/// Max in-flight classpath library fetches. Covers both the vanilla
/// library list (10–40 jars from `libraries.minecraft.net`) and the
/// loader "extras" (Fabric / Forge / NeoForge, another 10–20). We use
/// order-preserving `buffered(N)` here — classpath entry position and
/// same-key replacement semantics both depend on original list order.
const CLASSPATH_FETCH_CONCURRENCY: usize = 8;

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

    let java_distribution_path = cache.java_dir(java_version_component.as_ref());

    let java_distribution_path_ref = &java_distribution_path;
    futures::stream::iter(java_distribution.files)
        .map(|(path, fs_entry)| async move {
            if let FileSystemEntry::File {
                downloads,
                executable,
            } = fs_entry
            {
                let target_path = java_distribution_path_ref.join(path);
                cache
                    .materialize(&downloads.raw.url, Some(&downloads.raw.sha1), &target_path)
                    .await?;
                if executable {
                    make_executable(target_path).await?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .buffer_unordered(JAVA_FETCH_CONCURRENCY)
        .try_collect::<()>()
        .await?;

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
/// `forge_family_maven_base` is the fallback maven base for extra libraries
/// whose `downloads.artifact.url` is empty — the forge-family install
/// pipeline extracts those into a cache path derived from this base. `None`
/// when there are no forge-family extras (vanilla, or Fabric which always
/// populates `downloads.artifact.url`).
pub async fn build_class_path(
    version_manifest: &Version,
    client_jar: &Path,
    extra_libraries: &[LibraryEntry],
    forge_family_maven_base: Option<&Url>,
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

    // Fetch phase: parallel, order-preserving. Each future returns
    // `(classpath_entry, native_jar_path)` slots (either may be None
    // depending on the library entry shape and rules).
    type FetchedVanilla = (Option<(String, String)>, Option<PathBuf>);
    let vanilla_fetched: Vec<FetchedVanilla> =
        futures::stream::iter(&version_manifest.libraries)
            .map(|library_entry| async move {
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

                        rule_applies &= rule.features.is_none();

                        if rule_applies {
                            allow = rule.action == Action::Allow;
                        }
                    }
                }

                if !allow {
                    return Ok::<_, anyhow::Error>((None, None));
                }

                let cp = if let Some(artifact) = &library_entry.downloads.artifact
                    && let Some(url) = &artifact.url
                {
                    let path = cache.fetch(url, Some(&artifact.sha1)).await?;
                    Some((
                        dedup_key(&library_entry.name),
                        path.to_string_lossy().into_owned(),
                    ))
                } else {
                    None
                };

                let native_jar = if let Some(native) = &library_entry.downloads.classifiers
                    && let Some(native) = native.get(&NativeClassifier::current())
                    && let Some(url) = &native.url
                {
                    Some(cache.fetch(url, Some(&native.sha1)).await?)
                } else {
                    None
                };

                Ok((cp, native_jar))
            })
            .buffered(CLASSPATH_FETCH_CONCURRENCY)
            .try_collect()
            .await?;

    // Apply phase: sequential, in original order. `add_classpath_entry`
    // relies on insertion order for classpath position + same-key
    // replacement; zip extraction into `natives_directory` is a shared
    // synchronous writer we keep single-threaded on purpose.
    for (cp, native_jar) in vanilla_fetched {
        if let Some((key, path)) = cp {
            add_classpath_entry(&mut entries, &mut key_positions, key, path);
        }
        if let Some(jar_path) = native_jar {
            let jar_file = File::open(jar_path).await?;
            let mut archive = ZipArchive::new(jar_file.into_std().await)?;
            archive.extract(natives_directory)?;
        }
    }

    // Same shape as vanilla: parallel fetch, sequential apply. Order
    // preservation matters for the same dedup reasons.
    let extras_fetched: Vec<Option<(String, String)>> = futures::stream::iter(extra_libraries)
        .map(|lib| async move {
            if lib.clientreq == Some(false) {
                return Ok::<_, anyhow::Error>(None);
            }

            let path = if let Some(url) =
                lib.downloads.artifact.as_ref().and_then(|a| a.url.as_ref())
            {
                // Modern format: explicit download URL.
                let sha1 = lib.downloads.artifact.as_ref().map(|a| &a.sha1);
                cache.fetch(url, sha1).await?
            } else if let Some(artifact) = &lib.downloads.artifact {
                // downloads.artifact present but URL is empty: the JAR is bundled
                // in the Forge-family installer. forge::install() already extracted
                // it to a cache path derived from the flavor's maven base.
                let base = forge_family_maven_base.ok_or_else(|| {
                    anyhow::anyhow!(
                        "extra library {} has empty download URL but no forge-family \
                         maven base was passed to build_class_path — the caller must \
                         supply one when passing extras from a forge-family install",
                        lib.name
                    )
                })?;
                let url = base
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

            Ok(Some((
                dedup_key(&lib.name),
                path.to_string_lossy().into_owned(),
            )))
        })
        .buffered(CLASSPATH_FETCH_CONCURRENCY)
        .try_collect()
        .await?;

    for (key, path) in extras_fetched.into_iter().flatten() {
        add_classpath_entry(&mut entries, &mut key_positions, key, path);
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
