mod cache;
mod instance;

pub use instance::Instance;

use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;
use directories_next::ProjectDirs;
use reqwest::Client;
use tempfile::TempDir;
use tokio::{
    fs::{self, File},
    process::Command,
};
use url::Url;
use zip::ZipArchive;

use crate::api::{
    Endpoint,
    piston::{
        Action, FileSystemEntry, GameJarDownloadOptions, JavaDistribution,
        JavaDistributionManifest, JavaPlatform, JavaVersionComponent, NativeClassifier, Os,
        Version, VersionManifest,
    },
};
use cache::{AssetCache, Cache};

async fn make_executable(path: impl AsRef<Path>) -> anyhow::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o755)).await?;

    Ok(())
}

async fn fetch_version_manifest(cache: &Cache<'_>) -> anyhow::Result<VersionManifest> {
    cache.fetch_json(VersionManifest::url(), None).await
}

async fn fetch_java_distribution_manifest(
    cache: &Cache<'_>,
) -> anyhow::Result<JavaDistributionManifest> {
    let url = Url::from_str(
        "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json",
    )?;

    cache.fetch_json(&url, None).await
}

async fn fetch_java_distribution(
    java_version_component: JavaVersionComponent,
    cache: &Cache<'_>,
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
    cache: &Cache<'_>,
) -> anyhow::Result<PathBuf> {
    let download_entry = &game_jar_download_options.client;

    cache
        .fetch(&download_entry.url, Some(&download_entry.sha1))
        .await
}

async fn build_class_path(
    version_manifest: &Version,
    natives_directory: &TempDir,
    cache: &Cache<'_>,
) -> anyhow::Result<String> {
    let game_client_jar_path = fetch_game_client_jar(&version_manifest.downloads, &cache).await?;

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

pub struct Launcher {}

impl Launcher {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn launch(&self, instance: &Instance) -> anyhow::Result<()> {
        let Some(project_dirs) = ProjectDirs::from("", "", "cart") else {
            bail!("Could not find valid home directory path")
        };

        let cache_dir = project_dirs.cache_dir();

        let client = Client::new();
        let cache = Cache::new(cache_dir.to_path_buf(), &client);

        let version_manifest = fetch_version_manifest(&cache).await?;

        let version_map = version_manifest.version_map();

        let version_id = if instance.version() == "latest" {
            version_manifest.latest_release()
        } else {
            instance.version()
        };

        let version_info = &version_map.get(version_id).expect("unknown version");
        let version = cache
            .fetch_json::<Version>(&version_info.url, Some(&version_info.sha1))
            .await?;

        let asset_index = &version.asset_index;
        let asset_cache = AssetCache::new(&cache);
        asset_cache.update(asset_index).await?;
        let asset_directory = asset_cache.directory();

        let java_path = fetch_java_distribution(version.java_version.component, &cache).await?;

        let natives_directory = tempfile::tempdir()?;

        let classpath = build_class_path(&version, &natives_directory, &cache).await?;

        fs::create_dir_all(instance.directory()).await?;

        Command::new(java_path.join("bin").join("java"))
            .current_dir(instance.directory())
            .arg(format!(
                "-Djava.library.path={}",
                natives_directory.path().display()
            ))
            .arg("-Xmx4G")
            .arg("-Xms1G")
            .arg("-cp")
            .arg(classpath)
            .arg(version.main_class)
            .arg("--username")
            .arg("OfflinePlayer")
            .arg("--version")
            .arg(version.id)
            .arg("--gameDir")
            .arg(instance.directory())
            .arg("--assetsDir")
            .arg(asset_directory)
            .arg("--assetIndex")
            .arg(&asset_index.id)
            .arg("--uuid")
            .arg("00000000-0000-0000-0000-000000000000")
            .arg("--accessToken")
            .arg("0")
            .arg("--userType")
            .arg("legacy")
            .status()
            .await?;

        Ok(())
    }
}
