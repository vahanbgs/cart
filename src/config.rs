use std::path::{Path, PathBuf};

use crate::{Cli, Manifest};

pub struct Config<'cli> {
    cli: &'cli Cli,
    manifest: Manifest,
    manifest_directory: PathBuf,
}

impl<'cli> Config<'cli> {
    pub async fn load(cli: &'cli Cli) -> anyhow::Result<Self> {
        let (manifest_directory, manifest_path) = match cli.manifest_path() {
            Some(paths) => paths,
            None => Manifest::locate().await?,
        };

        let manifest = Manifest::load_from_path(&manifest_path).await?;

        Ok(Self {
            cli,
            manifest,
            manifest_directory,
        })
    }

    pub fn minecraft_version(&self) -> &str {
        self.cli
            .minecraft_version
            .as_deref()
            .unwrap_or(&self.manifest.minecraft)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn manifest_directory(&self) -> &Path {
        &self.manifest_directory
    }
}
