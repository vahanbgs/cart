mod document;
mod mod_dependency;

pub use document::{add_mod, load_document, remove_mod, save_document, set_mod_disabled};
pub use mod_dependency::ModDependency;

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub minecraft: String,
    pub forge: Option<String>,
    pub mods: HashMap<String, ModDependency>,
}

impl Manifest {
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
