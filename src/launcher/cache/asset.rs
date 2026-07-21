use std::path::{Path, PathBuf};

use tokio::fs;
use url::Url;

use crate::api::piston::{AssetIndex, AssetManifest};

use super::Cache;

pub struct AssetCache<'cache> {
    cache: &'cache Cache,
    directory: PathBuf,
}

impl<'cache> AssetCache<'cache> {
    pub fn new(cache: &'cache Cache) -> Self {
        let directory = cache.directory().join("assets");

        Self { cache, directory }
    }

    pub async fn update(&self, asset_index: &AssetIndex) -> anyhow::Result<()> {
        let asset_manifest = self
            .cache
            .fetch_json::<AssetManifest>(&asset_index.url, Some(&asset_index.sha1))
            .await?;

        let asset_manifest_path = self
            .cache
            .fetch(&asset_index.url, Some(&asset_index.sha1))
            .await?;

        let assets_indexes_path = self.directory().join("indexes");
        let assets_objects_path = self.directory().join("objects");

        fs::create_dir_all(&assets_indexes_path).await?;

        fs::copy(
            &asset_manifest_path,
            assets_indexes_path.join(asset_manifest_path.file_name().unwrap()),
        )
        .await?;

        let asset_store_url = Url::parse("https://resources.download.minecraft.net/")?;

        // Pre-1.6 asset indexes flag `map_to_resources: true`. MC reads
        // assets from `--assetsDir <virtual dir>` by relative name
        // (e.g. `sound/step/grass1.ogg`) rather than by content hash,
        // so we hardlink each object from the hash-based cache into
        // its name-based virtual path.
        let virtual_directory = asset_manifest
            .map_to_resources
            .then(|| self.directory().join("virtual").join(&asset_index.id));

        for (name, object) in &asset_manifest.objects {
            let digest = object.hash.to_hex();
            let first_byte = hex::encode(&object.hash.as_bytes()[..1]);

            let object_path = self
                .cache
                .fetch(
                    &asset_store_url
                        .join(&format!("{}/", &first_byte))?
                        .join(&digest)?,
                    Some(&object.hash),
                )
                .await?;

            if let Some(virtual_directory) = &virtual_directory {
                let virtual_path = virtual_directory.join(name);
                if !fs::try_exists(&virtual_path).await? {
                    if let Some(parent) = virtual_path.parent() {
                        fs::create_dir_all(parent).await?;
                    }
                    fs::hard_link(&object_path, &virtual_path).await?;
                }
            }
        }

        if !fs::try_exists(&assets_objects_path).await? {
            fs::symlink("../resources.download.minecraft.net/", &assets_objects_path).await?;
        }

        Ok(())
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
