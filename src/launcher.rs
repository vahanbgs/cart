mod cache;

use std::{
    collections::HashMap,
    env::{self},
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::bail;
use cart::{
    CartManifest, Cli,
    piston::{
        Action, AssetIndex, AssetManifest, FileSystemEntry, GameJarDownloadOptions,
        JavaDistributionListManifest, JavaDistributionManifest, JavaPlatform, JavaVersionComponent,
        NativeClassifier, Os, VersionInfo, VersionListManifest, VersionManifest,
    },
};
use clap::Parser;
use directories_next::ProjectDirs;
use reqwest::Client;
use tempfile::TempDir;
use tokio::{
    fs::{self, File},
    process::Command,
};
use url::Url;

use cache::Cache;
use zip::ZipArchive;

async fn make_executable(path: impl AsRef<Path>) -> anyhow::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o755)).await?;

    Ok(())
}

async fn fetch_version_list_manifest(cache: &Cache<'_>) -> anyhow::Result<VersionListManifest> {
    let url = Url::from_str("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")?;

    cache.fetch_json(&url, None).await
}

async fn fetch_java_distribution_list_manifest(
    cache: &Cache<'_>,
) -> anyhow::Result<JavaDistributionListManifest> {
    let url = Url::from_str(
        "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json",
    )?;

    cache.fetch_json(&url, None).await
}

async fn fetch_java_distribution(
    java_version_component: JavaVersionComponent,
    cache: &Cache<'_>,
) -> anyhow::Result<PathBuf> {
    let java_distribution_list_manifest = fetch_java_distribution_list_manifest(cache).await?;

    let java_distribution_info =
        &java_distribution_list_manifest[&JavaPlatform::CURRENT][&java_version_component][0];

    let java_distribution_manifest = cache
        .fetch_json::<JavaDistributionManifest>(
            &java_distribution_info.manifest.url,
            Some(&java_distribution_info.manifest.sha1),
        )
        .await?;

    let java_distribution_path = cache
        .directory()
        .join("java")
        .join(java_version_component.as_ref());

    for (path, fs_entry) in java_distribution_manifest.files {
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

async fn fetch_assets(asset_index: &AssetIndex, cache: &Cache<'_>) -> anyhow::Result<PathBuf> {
    let asset_manifest = cache
        .fetch_json::<AssetManifest>(&asset_index.url, Some(&asset_index.sha1))
        .await?;

    let assets_path = cache.directory().join("assets");
    let assets_indexes_path = assets_path.join("indexes");
    let assets_objects_path = assets_path.join("objects");

    fs::create_dir_all(&assets_indexes_path).await?;

    let asset_manifest_path = cache
        .fetch(&asset_index.url, Some(&asset_index.sha1))
        .await?;

    fs::copy(
        &asset_manifest_path,
        assets_indexes_path.join(asset_manifest_path.file_name().unwrap()),
    )
    .await?;

    let asset_store_url = Url::parse("https://resources.download.minecraft.net/")?;

    for (_, object) in asset_manifest.objects {
        let digest = object.hash.to_hex();
        let first_byte = hex::encode(&object.hash.as_bytes()[..1]);

        cache
            .fetch(
                &asset_store_url
                    .join(&format!("{}/", &first_byte))?
                    .join(&digest)?,
                Some(&object.hash),
            )
            .await?;
    }

    if !fs::try_exists(&assets_objects_path).await? {
        fs::symlink("../resources.download.minecraft.net/", &assets_objects_path).await?;
    }

    Ok(assets_path)
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
    version_manifest: &VersionManifest,
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

async fn try_find_manifest_file() -> anyhow::Result<PathBuf> {
    let mut current_directory = env::current_dir()?;

    loop {
        let manifest_path = current_directory.join("cart.toml");

        if fs::try_exists(&manifest_path).await? {
            return Ok(manifest_path);
        }

        if !current_directory.pop() {
            break;
        }
    }

    bail!("Could not find cart.toml manifest file");
}

async fn resolve_manifest_path(cli: &Cli) -> anyhow::Result<PathBuf> {
    if let Some(manifest_path) = &cli.manifest {
        return Ok(manifest_path.to_owned());
    }

    try_find_manifest_file().await
}

async fn load_manifest_file(path: &Path) -> anyhow::Result<CartManifest> {
    let manifest = toml::from_str(&fs::read_to_string(path).await?)?;

    Ok(manifest)
}

pub struct Launcher {}

impl Launcher {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn launch(&self) -> anyhow::Result<()> {
        let cli = Cli::parse();

        let manifest_path = resolve_manifest_path(&cli).await?;
        let mut cart_manifest = load_manifest_file(&manifest_path).await?;
        cart_manifest.override_with(&cli);
        let manifest_directory = manifest_path.parent().unwrap();

        let Some(project_dirs) = ProjectDirs::from("", "", "cart") else {
            bail!("Could not find valid home directory path")
        };

        let cache_dir = project_dirs.cache_dir();

        let client = Client::new();
        let cache = Cache::new(cache_dir.to_path_buf(), &client);

        let version_list_manifest = fetch_version_list_manifest(&cache).await?;

        let version_map = version_list_manifest
            .versions
            .iter()
            .map(|version| (version.id.to_owned(), version.to_owned()))
            .collect::<HashMap<String, VersionInfo>>();

        let version = if cart_manifest.minecraft_version() == "latest" {
            &version_list_manifest.latest.release
        } else {
            cart_manifest.minecraft_version()
        };

        let version_info = &version_map[version];
        let version_manifest = cache
            .fetch_json::<VersionManifest>(&version_info.url, Some(&version_info.sha1))
            .await?;

        let asset_index = &version_manifest.asset_index;

        let assets_path = fetch_assets(&asset_index, &cache).await?;
        let java_path =
            fetch_java_distribution(version_manifest.java_version.component, &cache).await?;

        let natives_directory = tempfile::tempdir()?;

        let classpath = build_class_path(&version_manifest, &natives_directory, &cache).await?;

        Command::new(java_path.join("bin").join("java"))
            .current_dir(manifest_directory)
            .arg(format!(
                "-Djava.library.path={}",
                natives_directory.path().display()
            ))
            .arg("-Xmx4G")
            .arg("-Xms1G")
            .arg("-cp")
            .arg(classpath)
            .arg(version_manifest.main_class)
            .arg("--username")
            .arg("OfflinePlayer")
            .arg("--version")
            .arg(version)
            .arg("--gameDir")
            .arg("minecraft/")
            .arg("--assetsDir")
            .arg(assets_path)
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
