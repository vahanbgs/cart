mod document;
mod mod_dependency;

pub use document::{
    add_curseforge_mod, add_modrinth_mod, load_document, remove_mod, save_document,
    set_mod_disabled, set_mod_file, set_mod_version,
};
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
    pub loader: Option<cart::Loader>,
    pub mods: HashMap<String, ModDependency>,
}

impl Manifest {
    pub async fn locate() -> anyhow::Result<(PathBuf, PathBuf)> {
        Self::locate_from(&env::current_dir()?).await
    }

    /// Walk up from `start` looking for `cart.toml`. Split out from
    /// `locate()` so tests can drive it against a tempdir without racing
    /// the process-wide cwd.
    pub async fn locate_from(start: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
        let mut current_directory = start.to_owned();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn locate_from_finds_manifest_in_start_dir() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("cart.toml");
        fs::write(&manifest_path, "minecraft = \"1.20.1\"\n[mods]\n")
            .await
            .unwrap();

        let (project, path) = Manifest::locate_from(dir.path()).await.unwrap();
        assert_eq!(project, dir.path());
        assert_eq!(path, manifest_path);
    }

    #[tokio::test]
    async fn locate_from_walks_up_to_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("cart.toml");
        fs::write(&manifest_path, "minecraft = \"1.20.1\"\n[mods]\n")
            .await
            .unwrap();
        let nested = dir.path().join("a/b/c");
        fs::create_dir_all(&nested).await.unwrap();

        let (project, path) = Manifest::locate_from(&nested).await.unwrap();
        assert_eq!(project, dir.path());
        assert_eq!(path, manifest_path);
    }

}
