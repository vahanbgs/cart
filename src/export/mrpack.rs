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

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
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

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
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
pub fn dependencies_from(minecraft: &str, loader: Option<&Loader>) -> anyhow::Result<Dependencies> {
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

/// Where a `[mods]` entry came from in cart.toml. Drives the routing
/// decision in [`build_entry`] — Modrinth and URL entries land in the
/// index's `files[]`; CurseForge entries are embedded as overrides so
/// the pack works even when the CF author has restricted third-party
/// downloads (in which case there's no URL Modrinth's ecosystem could
/// legally fetch from anyway).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModSource {
    Modrinth,
    Url,
    CurseForge,
}

/// One mod after cart's per-source resolution. Shared between the mrpack
/// and curseforge exporters — each format's `build_entry` reads only the
/// fields it needs.
pub struct ResolvedMod<'a> {
    pub source: ModSource,
    pub filename: &'a str,
    /// Local cached jar. `Some` whenever the driver called `fetch_mod`.
    /// `None` in one case: a CurseForge-source entry inside a CurseForge
    /// export, where the mod ships as a `{projectID, fileID}` reference
    /// and the jar bytes are never touched.
    pub cached_jar: Option<&'a Path>,
    /// `Some` for Modrinth (CDN URL) and URL entries. `Some`/`None`
    /// for CurseForge (`None` when the author disabled third-party API
    /// downloads).
    pub download_url: Option<&'a str>,
    /// The manifest's `disabled = true` bit. mrpack encodes this in the
    /// `.jar.disabled` filename suffix already; curseforge reads it to
    /// set `required: false` in `files[]`.
    pub disabled: bool,
    /// CurseForge project + file IDs, populated only for
    /// `ModSource::CurseForge`. The curseforge exporter drops them into
    /// `files[]` verbatim; mrpack ignores this field.
    pub curseforge_ids: Option<(u32, u32)>,
}

/// The routing decision for one mod. Callers pattern-match to either
/// push a `PackFile` into `PackIndex::files` or copy `source` into the
/// zip at `dest`.
#[derive(Debug)]
pub enum PackEntry {
    File(PackFile),
    Override {
        source: PathBuf,
        /// Full in-zip path — always under `overrides/`.
        dest: String,
    },
}

/// Decide where one mod lands in the exported pack.
///
/// * Modrinth / URL → `PackFile` (URL + hashes + size).
/// * CurseForge with a download URL → override (embed the jar; the CF
///   URL isn't on Modrinth's allowlist for `files[]`).
/// * CurseForge without a download URL → error naming the mod, so the
///   user can either remove it or swap to a redistributable source.
pub fn build_entry(m: &ResolvedMod<'_>) -> anyhow::Result<PackEntry> {
    match m.source {
        ModSource::Modrinth | ModSource::Url => {
            let url = m.download_url.with_context(|| {
                format!(
                    "mod '{}' has no download URL; cart can't put it in an mrpack",
                    m.filename
                )
            })?;
            let jar = m
                .cached_jar
                .with_context(|| format!("mod '{}' has no cached jar", m.filename))?;
            let file = pack_file_from(url, jar, m.filename)?;
            Ok(PackEntry::File(file))
        }
        ModSource::CurseForge => {
            m.download_url.with_context(|| {
                format!(
                    "CurseForge mod '{}' has no third-party download URL — the author \
                 disabled API redistribution. Can't include this mod in an mrpack; \
                 remove it from cart.toml or replace it with the Modrinth equivalent.",
                    m.filename
                )
            })?;
            let jar = m
                .cached_jar
                .with_context(|| format!("mod '{}' has no cached jar", m.filename))?;
            Ok(PackEntry::Override {
                source: jar.to_owned(),
                dest: format!("overrides/mods/{}", m.filename),
            })
        }
    }
}

/// Build a `PackFile` for one mod. Reads the cached jar to compute
/// SHA-1 and SHA-512 — mrpack requires both. Private because callers
/// should go through [`build_entry`] so the source routing stays in
/// one place.
fn pack_file_from(url: &str, jar: &Path, filename: &str) -> anyhow::Result<PackFile> {
    let bytes = std::fs::read(jar).with_context(|| format!("read cached jar {}", jar.display()))?;
    let sha1 = sha1_hex(&bytes);
    let sha512 = sha512_hex(&bytes);
    Ok(PackFile {
        path: format!("mods/{filename}"),
        hashes: Hashes { sha1, sha512 },
        // Every cart-managed mod is client-required; server-required
        // is a stronger claim than we have any way to verify from
        // cart.toml alone, so keep it optional. Users editing the
        // exported pack can strengthen it downstream.
        env: Some(Env {
            client: EnvValue::Required,
            server: EnvValue::Optional,
        }),
        downloads: vec![url.to_owned()],
        file_size: bytes.len() as u64,
    })
}

/// Serialize `index` as `modrinth.index.json` at the root of `output`
/// and stream each `overrides[i].1` file into the archive at `overrides[i].0`.
///
/// The mrpack format is a plain ZIP; `modrinth.index.json` must sit at
/// the archive root and every embedded file lives under `overrides/`.
/// Callers assemble the index + override list (routing per mod happens
/// in [`build_entry`]); this function only handles serialization and I/O.
pub fn write_pack(
    index: &PackIndex,
    overrides: &[(String, PathBuf)],
    output: &Path,
) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(output).with_context(|| format!("create {}", output.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let index_bytes = serde_json::to_vec_pretty(index).context("serialize modrinth.index.json")?;
    writer
        .start_file("modrinth.index.json", options)
        .context("start modrinth.index.json entry")?;
    writer
        .write_all(&index_bytes)
        .context("write modrinth.index.json bytes")?;

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

    writer.finish().context("finalize mrpack archive")?;
    Ok(())
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn sha512_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha512};
    let mut h = Sha512::new();
    h.update(bytes);
    hex::encode(h.finalize())
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
        let fixture_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mrpack/index_minimal.json");
        let expected = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));

        assert_eq!(actual.trim_end(), expected.trim_end());
    }

    // ── File-entry assembly ───────────────────────────────────────────

    /// FIPS 180-4 reference vector: SHA-512("abc"). If this ever fails,
    /// either the sha2 crate is broken or hex encoding got swapped for
    /// something exotic — both are catastrophic.
    #[test]
    fn sha512_hex_of_abc_matches_fips_vector() {
        assert_eq!(
            sha512_hex(b"abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        );
    }

    /// End-to-end for the pure helper: a tempfile with known bytes
    /// produces a `PackFile` with the FIPS hashes, the byte length as
    /// `fileSize`, the `mods/` path convention, and the URL passed
    /// straight through.
    #[test]
    fn pack_file_from_reads_hashes_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("example.jar");
        std::fs::write(&jar, b"abc").unwrap();

        let file = pack_file_from("https://example.com/example.jar", &jar, "example.jar").unwrap();

        assert_eq!(file.path, "mods/example.jar");
        assert_eq!(file.file_size, 3);
        assert_eq!(file.hashes.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            file.hashes.sha512,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        );
        assert_eq!(file.downloads, vec!["https://example.com/example.jar"]);
        assert_eq!(
            file.env,
            Some(Env {
                client: EnvValue::Required,
                server: EnvValue::Optional
            })
        );
    }

    fn write_tempjar(content: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("mod.jar");
        std::fs::write(&jar, content).unwrap();
        (dir, jar)
    }

    #[test]
    fn build_entry_modrinth_goes_to_files() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::Modrinth,
            filename: "mod.jar",
            cached_jar: Some(&jar),
            download_url: Some("https://cdn.modrinth.com/data/xxx/mod.jar"),
            disabled: false,
            curseforge_ids: None,
        };
        match build_entry(&m).unwrap() {
            PackEntry::File(f) => {
                assert_eq!(
                    f.downloads,
                    vec!["https://cdn.modrinth.com/data/xxx/mod.jar"]
                );
                assert_eq!(f.path, "mods/mod.jar");
            }
            PackEntry::Override { .. } => panic!("Modrinth entry should go in files[]"),
        }
    }

    #[test]
    fn build_entry_url_goes_to_files() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::Url,
            filename: "mod.jar",
            cached_jar: Some(&jar),
            download_url: Some("https://example.com/mod.jar"),
            disabled: false,
            curseforge_ids: None,
        };
        match build_entry(&m).unwrap() {
            PackEntry::File(f) => {
                assert_eq!(f.downloads, vec!["https://example.com/mod.jar"]);
            }
            PackEntry::Override { .. } => panic!("URL entry should go in files[]"),
        }
    }

    /// CF mods with a download URL are still routed to overrides —
    /// Modrinth's `files[]` allowlist doesn't include CF's CDN in
    /// general, so bundling the jar is the safe default.
    #[test]
    fn build_entry_curseforge_with_url_goes_to_overrides() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::CurseForge,
            filename: "mod.jar",
            cached_jar: Some(&jar),
            download_url: Some("https://mediafilez.forgecdn.net/files/xxx/mod.jar"),
            disabled: false,
            curseforge_ids: Some((238222, 8419086)),
        };
        match build_entry(&m).unwrap() {
            PackEntry::Override { source, dest } => {
                assert_eq!(source, jar);
                assert_eq!(dest, "overrides/mods/mod.jar");
            }
            PackEntry::File(_) => panic!("CF entry should go in overrides"),
        }
    }

    // ── ZIP writing ───────────────────────────────────────────────────

    /// Round-trip: write a small pack (index + one override), reopen it,
    /// and assert both members are present with the right bytes. Locks
    /// in the archive layout — `modrinth.index.json` at the root, every
    /// other file under the `overrides/` prefix — which is what makes
    /// the emitted archive an mrpack and not just a random ZIP.
    #[test]
    fn write_pack_round_trip() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("config.toml");
        std::fs::write(&src, b"hello=world\n").unwrap();

        let index = PackIndex {
            format_version: 1,
            game: "minecraft".to_owned(),
            version_id: "0.1.0".to_owned(),
            name: "round-trip".to_owned(),
            summary: None,
            files: vec![],
            dependencies: dependencies_from("1.20.1", None).unwrap(),
        };
        let output = dir.path().join("out.mrpack");
        let overrides = vec![("overrides/config/config.toml".to_owned(), src.clone())];

        write_pack(&index, &overrides, &output).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        // Index round-trips through serde back to an equal PackIndex.
        let mut index_bytes = Vec::new();
        archive
            .by_name("modrinth.index.json")
            .unwrap()
            .read_to_end(&mut index_bytes)
            .unwrap();
        let round_tripped: PackIndex = serde_json::from_slice(&index_bytes).unwrap();
        assert_eq!(round_tripped.name, "round-trip");
        assert_eq!(round_tripped.dependencies.minecraft, "1.20.1");
        assert!(round_tripped.files.is_empty());

        // Override lands at the exact declared path with unchanged bytes.
        let mut override_bytes = Vec::new();
        archive
            .by_name("overrides/config/config.toml")
            .unwrap()
            .read_to_end(&mut override_bytes)
            .unwrap();
        assert_eq!(override_bytes, b"hello=world\n");
    }

    /// The error must name the mod so the user knows which manifest
    /// entry to remove or swap, and mention CurseForge so the cause is
    /// obvious.
    #[test]
    fn build_entry_curseforge_without_url_errors_with_hint() {
        let (_dir, jar) = write_tempjar(b"abc");
        let m = ResolvedMod {
            source: ModSource::CurseForge,
            filename: "picky-mod.jar",
            cached_jar: Some(&jar),
            download_url: None,
            disabled: false,
            curseforge_ids: Some((238222, 8419086)),
        };
        let err = build_entry(&m).unwrap_err().to_string();
        assert!(
            err.contains("picky-mod.jar"),
            "expected filename in error: {err}"
        );
        assert!(
            err.contains("CurseForge"),
            "expected CurseForge in error: {err}"
        );
    }
}
