use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde::Deserialize;
use tokio::fs;

use crate::Cli;

#[derive(Debug, Deserialize)]
pub struct MinecraftVersion(String);

impl Default for MinecraftVersion {
    fn default() -> Self {
        Self("latest".to_owned())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    #[serde(default)]
    pub minecraft: MinecraftVersion,
}

impl Manifest {
    pub fn override_with(&mut self, cli: &Cli) {
        if let Some(minecraft_version) = &cli.minecraft_version {
            self.minecraft = MinecraftVersion(minecraft_version.to_owned());
        }
    }

    pub fn minecraft_version(&self) -> &str {
        &self.minecraft.0
    }

    async fn try_find_path() -> anyhow::Result<PathBuf> {
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

    pub async fn resolve_path(cli: &Cli) -> anyhow::Result<PathBuf> {
        if let Some(manifest_path) = &cli.manifest {
            return Ok(manifest_path.to_owned());
        }

        Self::try_find_path().await
    }

    async fn load_from_path(path: &Path) -> anyhow::Result<Manifest> {
        let manifest = toml::from_str(&fs::read_to_string(path).await?)?;

        Ok(manifest)
    }

    pub async fn load(cli: &Cli, path: &Path) -> anyhow::Result<Manifest> {
        let mut manifest = Self::load_from_path(path).await?;

        manifest.override_with(cli);

        Ok(manifest)
    }
}
