use std::path::{Path, PathBuf};

use tokio::fs;
use url::Url;

use crate::api::piston::{AssetIndex, AssetManifest};
use crate::launcher::fs_ops;
use crate::parallel;
use crate::progress::{self, IndicatifSpanExt};

use super::Cache;

pub struct AssetCache<'cache> {
    cache: &'cache Cache,
    directory: PathBuf,
}

impl<'cache> AssetCache<'cache> {
    pub fn new(cache: &'cache Cache) -> Self {
        let directory = cache.assets_dir();

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

        let virtual_directory = virtual_directory.as_ref();
        let asset_store_url = &asset_store_url;
        let bar = progress::bar("assets", asset_manifest.objects.len() as u64);
        bar.pb_start();
        let bar = &bar;
        parallel::run(&asset_manifest.objects, |(name, object)| async move {
            let digest = object.hash.to_hex();
            let first_byte = hex::encode(&object.hash.as_bytes()[..1]);
            let url = asset_store_url
                .join(&format!("{}/", &first_byte))?
                .join(&digest)?;

            if let Some(virtual_directory) = virtual_directory {
                let virtual_path = virtual_directory.join(name);
                self.cache
                    .materialize(&url, Some(&object.hash), &virtual_path)
                    .await?;
            } else {
                // Legacy asset indexes flag virtual materialization; modern
                // ones don't — but we still need to pull the object into the
                // cache so the game can resolve it via the assets/objects
                // symlink below.
                self.cache.fetch(&url, Some(&object.hash)).await?;
            }
            bar.pb_inc(1);
            Ok(())
        })
        .await?;

        fs_ops::symlink("../resources.download.minecraft.net/", &assets_objects_path).await?;

        Ok(())
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
