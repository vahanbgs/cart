//! `modrinth.index.json` schema and pure conversion helpers.
//!
//! The schema follows Modrinth's mrpack format v1
//! (<https://docs.modrinth.com/modpacks/format/>). Only the fields cart
//! emits are modelled — the format has room for other keys that
//! Modrinth's ecosystem-specific tooling reads but that a launcher-first
//! pack manager has no need to produce.
//!
//! Field ordering in the struct declarations mirrors what real mrpack
//! files look like on disk (`formatVersion` first, `dependencies` last,
//! `minecraft` before the loader key). Serde uses declaration order for
//! serialization, so the JSON we emit reads the way a human familiar
//! with mrpacks expects.

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::{Loader, LoaderKind, LoaderSpec};

#[derive(Debug, Deserialize, Serialize)]
pub struct PackIndex {
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub game: String,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub files: Vec<PackFile>,
    pub dependencies: Dependencies,
}

/// mrpack's `dependencies` block — Minecraft version plus at most one
/// loader key. Modelled as a struct with named optional loader fields
/// rather than a `Map<String, String>` so:
///   * cart can't accidentally emit multiple loader keys at once
///   * JSON output ordering matches real mrpack files (minecraft first,
///     then the loader)
///   * a rename or typo on the loader-key spelling (`fabric-loader`)
///     lands on tests here rather than at import time in Modrinth's
///     Prism-compatible tooling
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Dependencies {
    pub minecraft: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neoforge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fabric_loader: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PackFile {
    /// Destination path inside the extracted pack, relative to the
    /// game directory. `mods/foo.jar` for mods, `config/foo.toml` for
    /// configs.
    pub path: String,
    pub hashes: Hashes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    /// Ordered list of download URLs. Modrinth tries them in order;
    /// cart typically ships one (the Modrinth CDN URL or the URL from
    /// a `url = "..."` manifest entry).
    pub downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Hashes {
    pub sha1: String,
    pub sha512: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Env {
    pub client: EnvValue,
    pub server: EnvValue,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EnvValue {
    Required,
    Optional,
    Unsupported,
}

/// Build mrpack's `dependencies` block from cart's manifest inputs.
///
/// mrpack captures a specific pinned build, not a channel — a `Latest`
/// or `Recommended` loader spec would drift on every re-export, so we
/// reject those loudly and force the user to pin. `None` (vanilla)
/// produces a dependencies block with just `minecraft` set.
pub fn dependencies_from(
    minecraft: &str,
    loader: Option<&Loader>,
) -> anyhow::Result<Dependencies> {
    let mut deps = Dependencies {
        minecraft: minecraft.to_owned(),
        forge: None,
        neoforge: None,
        fabric_loader: None,
    };

    if let Some(loader) = loader {
        let version = match &loader.spec {
            LoaderSpec::Pinned(v) => v.clone(),
            LoaderSpec::Latest | LoaderSpec::Recommended => bail!(
                "mrpack export requires a pinned loader version; got {:?}. \
                 Change `loader` in cart.toml to a specific version like \
                 `loader = {{ forge = \"47.2.0\" }}`.",
                loader.spec
            ),
        };
        match loader.kind {
            LoaderKind::Forge => deps.forge = Some(version),
            LoaderKind::NeoForge => deps.neoforge = Some(version),
            LoaderKind::Fabric => deps.fabric_loader = Some(version),
        }
    }

    Ok(deps)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn forge_pinned(v: &str) -> Loader {
        Loader::forge(LoaderSpec::Pinned(v.to_owned()))
    }
    fn fabric_pinned(v: &str) -> Loader {
        Loader::fabric(LoaderSpec::Pinned(v.to_owned()))
    }
    fn neoforge_pinned(v: &str) -> Loader {
        Loader::neoforge(LoaderSpec::Pinned(v.to_owned()))
    }

    #[test]
    fn dependencies_vanilla_only_has_minecraft() {
        let deps = dependencies_from("1.20.1", None).unwrap();
        assert_eq!(deps.minecraft, "1.20.1");
        assert!(deps.forge.is_none());
        assert!(deps.neoforge.is_none());
        assert!(deps.fabric_loader.is_none());
    }

    #[test]
    fn dependencies_maps_forge() {
        let deps = dependencies_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap();
        assert_eq!(deps.minecraft, "1.20.1");
        assert_eq!(deps.forge.as_deref(), Some("47.2.0"));
    }

    /// mrpack's loader key for Fabric is `fabric-loader` (kebab), not
    /// `fabric` — a common footgun; lock it in.
    #[test]
    fn dependencies_maps_fabric_to_fabric_loader_key() {
        let deps = dependencies_from("1.20.1", Some(&fabric_pinned("0.15.7"))).unwrap();
        assert_eq!(deps.fabric_loader.as_deref(), Some("0.15.7"));
        // Round-trip through JSON to catch the kebab-rename regressing.
        let json = serde_json::to_string(&deps).unwrap();
        assert!(
            json.contains("\"fabric-loader\":\"0.15.7\""),
            "expected fabric-loader key in JSON: {json}"
        );
    }

    #[test]
    fn dependencies_maps_neoforge() {
        let deps = dependencies_from("1.21.1", Some(&neoforge_pinned("21.1.242"))).unwrap();
        assert_eq!(deps.neoforge.as_deref(), Some("21.1.242"));
    }

    /// mrpack captures a concrete build; channels drift on re-export.
    /// Both channel specs must fail with a message that points the user
    /// at cart.toml.
    #[test]
    fn dependencies_rejects_latest_channel() {
        let loader = Loader::forge(LoaderSpec::Latest);
        let err = dependencies_from("1.20.1", Some(&loader)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("pinned"), "expected pin hint in error: {msg}");
        assert!(
            msg.contains("cart.toml"),
            "expected cart.toml hint in error: {msg}"
        );
    }

    #[test]
    fn dependencies_rejects_recommended_channel() {
        let loader = Loader::forge(LoaderSpec::Recommended);
        let err = dependencies_from("1.20.1", Some(&loader)).unwrap_err();
        assert!(err.to_string().contains("pinned"));
    }

    /// Golden-JSON test: hand-build a small `PackIndex`, serialize it,
    /// and compare against a checked-in fixture. Locks in field
    /// spellings (`formatVersion`, `versionId`, `fileSize`), field
    /// ordering, and `skip_serializing_if` on the optionals. A schema
    /// drift here means an mrpack cart emits stops parsing in Modrinth's
    /// tooling.
    #[test]
    fn hand_built_index_matches_golden() {
        let index = PackIndex {
            format_version: 1,
            game: "minecraft".to_owned(),
            version_id: "1.0.0".to_owned(),
            name: "hand-built".to_owned(),
            summary: Some("test fixture".to_owned()),
            files: vec![PackFile {
                path: "mods/example.jar".to_owned(),
                hashes: Hashes {
                    sha1: "aabbccddeeff00112233445566778899aabbccdd".to_owned(),
                    sha512: "00".repeat(64),
                },
                env: Some(Env {
                    client: EnvValue::Required,
                    server: EnvValue::Optional,
                }),
                downloads: vec!["https://example.com/example.jar".to_owned()],
                file_size: 1234,
            }],
            dependencies: dependencies_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap(),
        };

        let actual = serde_json::to_string_pretty(&index).unwrap();
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mrpack/index_minimal.json");
        let expected = std::fs::read_to_string(&fixture_path).unwrap_or_else(|e| {
            panic!("read {}: {e}", fixture_path.display())
        });

        assert_eq!(actual.trim_end(), expected.trim_end());
    }
}
