mod document;
mod mod_dependency;

pub use document::{
    add_curseforge_mod, add_modrinth_mod, load_document, remove_mod, save_document,
    set_mod_disabled, set_mod_file, set_mod_version,
};
pub use mod_dependency::ModDependency;

use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
};

use anyhow::bail;
use serde::Deserialize;
use tokio::fs;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Human-readable pack name. Required at export time (all three
    /// export formats bake it in); `cart run` doesn't care.
    pub name: Option<String>,

    /// Pack version string (e.g. "1.0.0"). Required at export time —
    /// used as `versionId` in mrpack, `version` in curseforge, and the
    /// suggested output filename. Not validated as semver; whatever
    /// convention the pack author picks is fine.
    pub version: Option<String>,

    /// Pack authors. Cargo-style plural even when there's one — CF's
    /// `manifest.json` takes a single `author` string, so cart joins
    /// with `, ` at export time when needed.
    #[serde(default)]
    pub authors: Vec<String>,

    /// Short pack description. Optional in all three export formats.
    pub summary: Option<String>,

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

    /// Slugs of every Modrinth-sourced entry in `[mods]`. URL and
    /// CurseForge entries are skipped. Used by `mr search` / `mr find`
    /// to hide already-declared mods from search results.
    pub fn modrinth_slugs(&self) -> HashSet<&str> {
        self.mods
            .values()
            .filter_map(|m| match m {
                ModDependency::Modrinth { modrinth, .. } => Some(modrinth.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Project ids of every CurseForge-sourced entry in `[mods]`. URL
    /// and Modrinth entries are skipped. Used by `cf search` / `cf find`
    /// to hide already-declared mods from search results.
    pub fn curseforge_project_ids(&self) -> HashSet<u32> {
        self.mods
            .values()
            .filter_map(|m| match m {
                ModDependency::CurseForge { curseforge, .. } => Some(*curseforge),
                _ => None,
            })
            .collect()
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

    /// The oldest cart.toml shape — only `minecraft` + `[mods]` — must
    /// keep parsing so existing packs don't break when they land on a
    /// cart that knows about the new export fields.
    #[test]
    fn parses_manifest_without_pack_fields() {
        let toml = r#"
minecraft = "1.20.1"
[mods]
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(m.name.is_none());
        assert!(m.version.is_none());
        assert!(m.authors.is_empty());
        assert!(m.summary.is_none());
        assert_eq!(m.minecraft, "1.20.1");
    }

    /// Every export-time pack field populated at once. Locks in the
    /// spelling — a rename here would silently make every existing
    /// exportable pack invalid.
    #[test]
    fn parses_manifest_with_all_pack_fields() {
        let toml = r#"
name = "my-pack"
version = "1.2.3"
authors = ["alice", "bob"]
summary = "a demo pack"
minecraft = "1.20.1"
loader = { forge = "47.2.0" }
[mods]
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.name.as_deref(), Some("my-pack"));
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert_eq!(m.authors, vec!["alice", "bob"]);
        assert_eq!(m.summary.as_deref(), Some("a demo pack"));
    }

    #[test]
    fn modrinth_slugs_collects_only_modrinth_entries() {
        let toml = r#"
minecraft = "1.20.1"
[mods]
jei = { modrinth = "jei" }
appleskin = { modrinth = "appleskin", disabled = true }
custom = { url = "https://example.com/x.jar" }
cfmod = { curseforge = 238222, file = 8419086 }
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        let slugs = m.modrinth_slugs();
        assert_eq!(slugs.len(), 2);
        assert!(slugs.contains("jei"));
        assert!(slugs.contains("appleskin"));
    }

    #[test]
    fn curseforge_project_ids_collects_only_curseforge_entries() {
        let toml = r#"
minecraft = "1.20.1"
[mods]
jei = { modrinth = "jei" }
cfmod = { curseforge = 238222, file = 8419086 }
cfdis = { curseforge = 999, file = 111, disabled = true }
custom = { url = "https://example.com/x.jar" }
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        let ids = m.curseforge_project_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&238222));
        assert!(ids.contains(&999));
    }

    #[test]
    fn accessors_return_empty_on_empty_mods_table() {
        let toml = r#"
minecraft = "1.20.1"
[mods]
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert!(m.modrinth_slugs().is_empty());
        assert!(m.curseforge_project_ids().is_empty());
    }

    /// `authors` is Cargo-style plural even when only one; a bare
    /// string (`authors = "alice"`) is a schema error and must fail
    /// loudly rather than silently coercing.
    #[test]
    fn authors_bare_string_is_rejected() {
        let toml = r#"
name = "p"
version = "0"
authors = "alice"
minecraft = "1.20.1"
[mods]
"#;
        assert!(toml::from_str::<Manifest>(toml).is_err());
    }
}
