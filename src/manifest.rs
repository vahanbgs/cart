mod mod_dependency;

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde::{Deserialize, Serialize};
use tokio::fs;

use mod_dependency::ModDependency;

use crate::Cli;

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub minecraft: String,
    pub mods: HashMap<String, ModDependency>,
}

impl Manifest {
    pub fn new(cli: &Cli) -> Self {
        Self {
            minecraft: cli
                .minecraft_version
                .as_deref()
                .unwrap_or("latest")
                .to_owned(),
            mods: Default::default(),
        }
    }

    pub async fn locate() -> anyhow::Result<(PathBuf, PathBuf)> {
        let mut current_directory = env::current_dir()?;

        loop {
            let manifest_path = current_directory.join("cart.toml");

            if fs::try_exists(&manifest_path).await? {
                return Ok((current_directory, manifest_path));
            }

            if !current_directory.pop() {
                break;
            }
        }

        bail!("Could not find cart.toml manifest file");
    }

    pub async fn load_from_path(path: &Path) -> anyhow::Result<Manifest> {
        let manifest = toml::from_str(&fs::read_to_string(path).await?)?;

        Ok(manifest)
    }
}
