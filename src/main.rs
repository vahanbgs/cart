mod cache;

use std::{
    collections::HashMap, fs::Permissions, os::unix::fs::PermissionsExt, path::Path, str::FromStr,
};

use anyhow::bail;
use cart::piston::{
    AssetIndex, AssetManifest, FileSystemEntry, JavaDistributionListManifest,
    JavaDistributionManifest, JavaPlatform, JavaVersionComponent, VersionInfo, VersionListManifest,
    VersionManifest,
};
use directories_next::ProjectDirs;
use reqwest::Client;
use tokio::fs;
use url::Url;

use cache::Cache;

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

async fn download_java_distribution(
    java_version_component: JavaVersionComponent,
    cache: &Cache<'_>,
) -> anyhow::Result<()> {
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
        .path()
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

    Ok(())
}

async fn download_assets(asset_index: &AssetIndex, cache: &Cache<'_>) -> anyhow::Result<()> {
    let asset_manifest = cache
        .fetch_json::<AssetManifest>(&asset_index.url, Some(&asset_index.sha1))
        .await?;

    let assets_path = cache.path().join("assets");
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

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let version = "26.2";

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

    let version_info = &version_map[version];
    let version_manifest = cache
        .fetch_json::<VersionManifest>(&version_info.url, Some(&version_info.sha1))
        .await?;

    download_assets(&version_manifest.asset_index, &cache).await?;
    download_java_distribution(version_manifest.java_version.component, &cache).await?;

    // let mut classpath = Vec::new();

    // for library_entry in version_manifest.libraries {
    //     fetch(
    //         library_entry.downloads.artifact,
    //         cache_dir,
    //         &client,
    //         expected_digest,
    //     )
    //     .await?;
    //     let Some(rules) = library_entry.rules else {};
    // }

    Ok(())
}
