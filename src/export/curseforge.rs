//! CurseForge modpack `manifest.json` schema and pure conversion helpers.
//!
//! The schema follows the CurseForge modpack format used by the official
//! CurseForge launcher, ATLauncher, and CF-pack-aware tools. Only the
//! fields cart emits are modelled — the format has room for extra keys
//! (e.g. `modlist.html`, `overrides` name overrides) that a launcher
//! producing packs from a lockfile-like manifest has no need to touch.
//!
//! Field ordering in the struct declarations mirrors what real
//! `manifest.json` files look like on disk — serde uses declaration
//! order for serialization, so the JSON we emit reads the way a human
//! familiar with CF packs expects. The routing rule is the mirror image
//! of mrpack: CurseForge-sourced mods land in `files[]` as
//! `{projectID, fileID, required}`; Modrinth-sourced and URL-sourced
//! mods have no CF ID and ship as `overrides/mods/…` embeds.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Serialize;

use crate::{
    Loader, LoaderKind, LoaderSpec,
    export::mrpack::{ModSource, ResolvedMod},
};

#[derive(Debug, Serialize)]
pub struct CurseForgeManifest {
    pub minecraft: MinecraftBlock,
    #[serde(rename = "manifestType")]
    pub manifest_type: &'static str,
    #[serde(rename = "manifestVersion")]
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    pub author: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub files: Vec<CfFile>,
    pub overrides: &'static str,
}

impl CurseForgeManifest {
    /// Constant `manifestType` field value; every CF modpack uses it.
    pub const MANIFEST_TYPE: &'static str = "minecraftModpack";
    /// Constant `overrides` field value; the folder inside the archive
    /// where embedded files live. Real packs occasionally rename this
    /// but there's no reason cart should.
    pub const OVERRIDES_DIR: &'static str = "overrides";
}

#[derive(Debug, Serialize)]
pub struct MinecraftBlock {
    pub version: String,
    #[serde(rename = "modLoaders")]
    pub mod_loaders: Vec<CfModLoader>,
}

/// One entry in `minecraft.modLoaders`. CF's id-string form is
/// `<loader>-<version>` (e.g. `forge-47.2.0`, `fabric-0.15.7`,
/// `neoforge-21.1.242`). Exactly one loader is marked `primary`; cart
/// only ever emits one so it's always `true`.
#[derive(Debug, Serialize)]
pub struct CfModLoader {
    pub id: String,
    pub primary: bool,
}

#[derive(Debug, Serialize)]
pub struct CfFile {
    #[serde(rename = "projectID")]
    pub project_id: u32,
    #[serde(rename = "fileID")]
    pub file_id: u32,
    pub required: bool,
}

/// The routing decision for one mod. Callers pattern-match to either
/// push a `CfFile` into `CurseForgeManifest::files` or copy `source`
/// into the zip at `dest`.
#[derive(Debug)]
pub enum CfPackEntry {
    File(CfFile),
    Override {
        source: PathBuf,
        /// Full in-zip path — always under `overrides/`.
        dest: String,
    },
}

/// Build the `minecraft` block from cart's manifest inputs.
///
/// Same reasoning as `mrpack::dependencies_from`: a channel spec
/// (`Latest`/`Recommended`) would drift on every re-export, so we
/// reject those loudly and force the user to pin. `None` (vanilla)
/// produces an empty `modLoaders[]`, which CF importers accept.
pub fn minecraft_block_from(
    minecraft: &str,
    loader: Option<&Loader>,
) -> anyhow::Result<MinecraftBlock> {
    let mod_loaders = match loader {
        None => vec![],
        Some(loader) => {
            let version = match &loader.spec {
                LoaderSpec::Pinned(v) => v.clone(),
                LoaderSpec::Latest | LoaderSpec::Recommended => bail!(
                    "curseforge export requires a pinned loader version; got {:?}. \
                     Change `loader` in cart.toml to a specific version like \
                     `loader = {{ forge = \"47.2.0\" }}`.",
                    loader.spec
                ),
            };
            let prefix = match loader.kind {
                LoaderKind::Forge => "forge",
                LoaderKind::Fabric => "fabric",
                LoaderKind::NeoForge => "neoforge",
            };
            vec![CfModLoader {
                id: format!("{prefix}-{version}"),
                primary: true,
            }]
        }
    };
    Ok(MinecraftBlock {
        version: minecraft.to_owned(),
        mod_loaders,
    })
}

/// Decide where one mod lands in the exported pack. Inverted from
/// mrpack: CurseForge is native (goes in `files[]`); Modrinth and URL
/// have no CF IDs and must be embedded as overrides.
///
/// Disabled CF-source entries emit `required: false` — CF launchers
/// present those as optional/off at install time. Disabled
/// Modrinth/URL entries keep the `.jar.disabled` suffix in `filename`
/// so the embedded file lands at `overrides/mods/foo.jar.disabled`
/// (installed but not loaded by the game).
pub fn build_entry(m: &ResolvedMod<'_>) -> anyhow::Result<CfPackEntry> {
    match m.source {
        ModSource::CurseForge => {
            let (project_id, file_id) = m.curseforge_ids.with_context(|| {
                format!(
                    "CurseForge mod '{}' is missing project/file IDs — the CLI \
                     didn't fill in `curseforge_ids` on ResolvedMod",
                    m.filename
                )
            })?;
            Ok(CfPackEntry::File(CfFile {
                project_id,
                file_id,
                required: !m.disabled,
            }))
        }
        ModSource::Modrinth | ModSource::Url => {
            let jar = m
                .cached_jar
                .with_context(|| format!("mod '{}' has no cached jar", m.filename))?;
            Ok(CfPackEntry::Override {
                source: jar.to_owned(),
                dest: format!("overrides/mods/{}", m.filename),
            })
        }
    }
}

/// Serialize `manifest` as `manifest.json` at the root of `output` and
/// stream each `overrides[i].1` file into the archive at `overrides[i].0`.
///
/// The CurseForge modpack format is a plain ZIP; `manifest.json` must
/// sit at the archive root and every embedded file lives under
/// `overrides/`. Same layout logic as [`crate::export::mrpack::write_pack`];
/// only the root filename and manifest shape differ.
pub fn write_pack(
    manifest: &CurseForgeManifest,
    overrides: &[(String, PathBuf)],
    output: &Path,
) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(output).with_context(|| format!("create {}", output.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let manifest_bytes = serde_json::to_vec_pretty(manifest).context("serialize manifest.json")?;
    writer
        .start_file("manifest.json", options)
        .context("start manifest.json entry")?;
    writer
        .write_all(&manifest_bytes)
        .context("write manifest.json bytes")?;

    for (dest, source) in overrides {
        let bytes = std::fs::read(source)
            .with_context(|| format!("read override source {}", source.display()))?;
        writer
            .start_file(dest, options)
            .with_context(|| format!("start ZIP entry {dest}"))?;
        writer
            .write_all(&bytes)
            .with_context(|| format!("write ZIP entry {dest}"))?;
    }

    writer.finish().context("finalize CurseForge modpack archive")?;
    Ok(())
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
    fn minecraft_block_vanilla_has_empty_mod_loaders() {
        let mc = minecraft_block_from("1.20.1", None).unwrap();
        assert_eq!(mc.version, "1.20.1");
        assert!(mc.mod_loaders.is_empty());
    }

    #[test]
    fn minecraft_block_maps_forge_to_prefixed_id() {
        let mc = minecraft_block_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap();
        assert_eq!(mc.mod_loaders.len(), 1);
        assert_eq!(mc.mod_loaders[0].id, "forge-47.2.0");
        assert!(mc.mod_loaders[0].primary);
    }

    #[test]
    fn minecraft_block_maps_fabric_to_prefixed_id() {
        let mc = minecraft_block_from("1.20.1", Some(&fabric_pinned("0.15.7"))).unwrap();
        assert_eq!(mc.mod_loaders[0].id, "fabric-0.15.7");
    }

    #[test]
    fn minecraft_block_maps_neoforge_to_prefixed_id() {
        let mc = minecraft_block_from("1.21.1", Some(&neoforge_pinned("21.1.242"))).unwrap();
        assert_eq!(mc.mod_loaders[0].id, "neoforge-21.1.242");
    }

    #[test]
    fn minecraft_block_rejects_latest_channel() {
        let loader = Loader::forge(LoaderSpec::Latest);
        let err = minecraft_block_from("1.20.1", Some(&loader)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("pinned"), "expected pin hint in error: {msg}");
        assert!(
            msg.contains("cart.toml"),
            "expected cart.toml hint in error: {msg}"
        );
    }

    #[test]
    fn minecraft_block_rejects_recommended_channel() {
        let loader = Loader::forge(LoaderSpec::Recommended);
        let err = minecraft_block_from("1.20.1", Some(&loader)).unwrap_err();
        assert!(err.to_string().contains("pinned"));
    }

    // ── Per-mod routing ──────────────────────────────────────────────

    fn write_tempjar(content: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("mod.jar");
        std::fs::write(&jar, content).unwrap();
        (dir, jar)
    }

    #[test]
    fn build_entry_curseforge_goes_to_files_with_ids() {
        let m = ResolvedMod {
            source: ModSource::CurseForge,
            filename: "jei.jar",
            cached_jar: None,
            download_url: None,
            disabled: false,
            curseforge_ids: Some((238222, 8419086)),
        };
        match build_entry(&m).unwrap() {
            CfPackEntry::File(f) => {
                assert_eq!(f.project_id, 238222);
                assert_eq!(f.file_id, 8419086);
                assert!(f.required);
            }
            CfPackEntry::Override { .. } => panic!("CF entry should go in files[]"),
        }
    }

    /// Disabled CF entries stay in `files[]` but flip `required` off so
    /// CF launchers surface them as optional/off — matches cart's
    /// `disabled = true` semantic without needing to embed the jar.
    #[test]
    fn build_entry_curseforge_disabled_is_not_required() {
        let m = ResolvedMod {
            source: ModSource::CurseForge,
            filename: "jei.jar.disabled",
            cached_jar: None,
            download_url: None,
            disabled: true,
            curseforge_ids: Some((238222, 8419086)),
        };
        match build_entry(&m).unwrap() {
            CfPackEntry::File(f) => assert!(!f.required),
            CfPackEntry::Override { .. } => panic!("CF entry should go in files[]"),
        }
    }

    #[test]
    fn build_entry_modrinth_goes_to_overrides() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::Modrinth,
            filename: "appleskin.jar",
            cached_jar: Some(&jar),
            download_url: Some("https://cdn.modrinth.com/data/xxx/appleskin.jar"),
            disabled: false,
            curseforge_ids: None,
        };
        match build_entry(&m).unwrap() {
            CfPackEntry::Override { source, dest } => {
                assert_eq!(source, jar);
                assert_eq!(dest, "overrides/mods/appleskin.jar");
            }
            CfPackEntry::File(_) => panic!("Modrinth entry should go in overrides"),
        }
    }

    #[test]
    fn build_entry_url_goes_to_overrides() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::Url,
            filename: "custom.jar",
            cached_jar: Some(&jar),
            download_url: Some("https://example.com/custom.jar"),
            disabled: false,
            curseforge_ids: None,
        };
        match build_entry(&m).unwrap() {
            CfPackEntry::Override { dest, .. } => {
                assert_eq!(dest, "overrides/mods/custom.jar");
            }
            CfPackEntry::File(_) => panic!("URL entry should go in overrides"),
        }
    }

    /// Disabled Modrinth entries preserve the `.jar.disabled` suffix in
    /// the override dest, same convention mrpack uses. That's how the
    /// launcher installs the mod as present-but-off without any
    /// per-entry metadata.
    #[test]
    fn build_entry_modrinth_disabled_preserves_suffix() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::Modrinth,
            filename: "opt.jar.disabled",
            cached_jar: Some(&jar),
            download_url: Some("https://cdn.modrinth.com/data/xxx/opt.jar"),
            disabled: true,
            curseforge_ids: None,
        };
        match build_entry(&m).unwrap() {
            CfPackEntry::Override { dest, .. } => {
                assert_eq!(dest, "overrides/mods/opt.jar.disabled");
            }
            CfPackEntry::File(_) => panic!("Modrinth entry should go in overrides"),
        }
    }

    // ── JSON layout ──────────────────────────────────────────────────

    /// Golden-JSON test: hand-build a small `CurseForgeManifest`,
    /// serialize it, and compare against a checked-in fixture. Locks in
    /// field spellings (`projectID`, `fileID`, `manifestType`,
    /// `manifestVersion`, `modLoaders`), field ordering, and
    /// `skip_serializing_if` on `description`. A schema drift here
    /// means a `manifest.json` cart emits stops parsing in the CF
    /// launcher ecosystem.
    #[test]
    fn hand_built_manifest_matches_golden() {
        let manifest = CurseForgeManifest {
            minecraft: minecraft_block_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap(),
            manifest_type: CurseForgeManifest::MANIFEST_TYPE,
            manifest_version: 1,
            name: "hand-built".to_owned(),
            version: "1.0.0".to_owned(),
            author: "alice, bob".to_owned(),
            description: "test fixture".to_owned(),
            files: vec![CfFile {
                project_id: 238222,
                file_id: 8419086,
                required: true,
            }],
            overrides: CurseForgeManifest::OVERRIDES_DIR,
        };

        let actual = serde_json::to_string_pretty(&manifest).unwrap();
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/curseforge/manifest_minimal.json");
        let expected = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));

        assert_eq!(actual.trim_end(), expected.trim_end());
    }

    /// When `description` (cart's `summary`) is empty,
    /// `skip_serializing_if` drops the field entirely — real CF packs
    /// without a description just omit the key rather than emit `""`.
    #[test]
    fn empty_description_is_omitted() {
        let manifest = CurseForgeManifest {
            minecraft: minecraft_block_from("1.20.1", None).unwrap(),
            manifest_type: CurseForgeManifest::MANIFEST_TYPE,
            manifest_version: 1,
            name: "no-desc".to_owned(),
            version: "1.0.0".to_owned(),
            author: String::new(),
            description: String::new(),
            files: vec![],
            overrides: CurseForgeManifest::OVERRIDES_DIR,
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(
            !json.contains("description"),
            "expected description to be omitted: {json}"
        );
    }

    // ── ZIP writing ──────────────────────────────────────────────────

    #[test]
    fn write_pack_round_trip() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, b"hello=world\n").unwrap();

        let manifest = CurseForgeManifest {
            minecraft: minecraft_block_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap(),
            manifest_type: CurseForgeManifest::MANIFEST_TYPE,
            manifest_version: 1,
            name: "round-trip".to_owned(),
            version: "0.1.0".to_owned(),
            author: "alice".to_owned(),
            description: String::new(),
            files: vec![CfFile {
                project_id: 1,
                file_id: 2,
                required: true,
            }],
            overrides: CurseForgeManifest::OVERRIDES_DIR,
        };
        let output = dir.path().join("out.zip");
        let overrides = vec![("overrides/config/config.toml".to_owned(), src)];

        write_pack(&manifest, &overrides, &output).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        let mut manifest_bytes = Vec::new();
        archive
            .by_name("manifest.json")
            .unwrap()
            .read_to_end(&mut manifest_bytes)
            .unwrap();
        let round_tripped: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(round_tripped["manifestType"], "minecraftModpack");
        assert_eq!(round_tripped["manifestVersion"], 1);
        assert_eq!(round_tripped["name"], "round-trip");
        assert_eq!(round_tripped["minecraft"]["version"], "1.20.1");
        assert_eq!(
            round_tripped["minecraft"]["modLoaders"][0]["id"],
            "forge-47.2.0"
        );
        assert_eq!(round_tripped["files"][0]["projectID"], 1);
        assert_eq!(round_tripped["files"][0]["fileID"], 2);

        let mut override_bytes = Vec::new();
        archive
            .by_name("overrides/config/config.toml")
            .unwrap()
            .read_to_end(&mut override_bytes)
            .unwrap();
        assert_eq!(override_bytes, b"hello=world\n");
    }
}
