use std::path::{Path, PathBuf};

use tokio::fs;
use url::Url;

use crate::api::piston::{AssetIndex, AssetManifest};

use super::Cache;

pub struct AssetCache<'cache, 'client> {
    cache: &'cache Cache<'client>,
    directory: PathBuf,
}

impl<'cache, 'client> AssetCache<'cache, 'client> {
    pub fn new(cache: &'cache Cache<'client>) -> Self {
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

        for (_, object) in asset_manifest.objects {
            let digest = object.hash.to_hex();
            let first_byte = hex::encode(&object.hash.as_bytes()[..1]);

            self.cache
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

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
