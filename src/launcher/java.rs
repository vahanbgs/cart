use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use tokio::{
    fs::{self, File},
    process::Command,
};
use zip::ZipArchive;

use crate::api::piston::{
    Action, FileSystemEntry, GameJarDownloadOptions, JavaDistribution, JavaDistributionManifest,
    JavaPlatform, JavaVersionComponent, NativeClassifier, Os, Version,
};

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

async fn fetch_game_client_jar(
    game_jar_download_options: &GameJarDownloadOptions,
    cache: &Cache,
) -> anyhow::Result<PathBuf> {
    let download_entry = &game_jar_download_options.client;

    cache
        .fetch(&download_entry.url, Some(&download_entry.sha1))
        .await
}

pub async fn build_class_path(
    version_manifest: &Version,
    natives_directory: &TempDir,
    cache: &Cache,
) -> anyhow::Result<String> {
    let game_client_jar_path = fetch_game_client_jar(&version_manifest.downloads, cache).await?;

    let mut classpath = vec![game_client_jar_path.to_string_lossy().into_owned()];

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
            let path = cache.fetch(&artifact.url, Some(&artifact.sha1)).await?;
            classpath.push(path.to_string_lossy().into_owned());
        }

        if let Some(native) = &library_entry.downloads.classifiers {
            if let Some(native) = native.get(&NativeClassifier::current()) {
                let jar_path = cache.fetch(&native.url, Some(&native.sha1)).await?;
                let jar_file = File::open(jar_path).await?;
                let mut archive = ZipArchive::new(jar_file.into_std().await)?;

                archive.extract(natives_directory)?;
            }
        }
    }

    Ok(classpath.join(":"))
}

pub fn java_binary(java_path: &Path) -> Command {
    Command::new(java_path.join("bin").join("java"))
}
