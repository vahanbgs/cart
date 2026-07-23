//! PrismLauncher / MultiMC instance ZIP schema and pure conversion
//! helpers.
//!
//! A Prism instance archive is a plain ZIP whose single top-level
//! folder is the instance directory. Inside that folder live:
//! - `instance.cfg`: an INI-ish key=value file (`InstanceType=OneSix`,
//!   `name=…`, optional `notes=…`).
//! - `mmc-pack.json`: a `components` list (Minecraft + optional loader)
//!   with `formatVersion: 1`. Prism resolves LWJGL, libraries, and
//!   any other dependencies on-launch — cart emits only the components
//!   whose versions the user chose.
//! - `.minecraft/mods/*.jar`: every mod jar, embedded (Prism instances
//!   are self-contained; no URL indirection).
//! - `.minecraft/…`: cart's `overrides/` tree mirrored here for configs,
//!   resourcepacks, kubejs scripts, etc.
//!
//! The routing model differs from mrpack/curseforge: there's no
//! per-mod `File`-vs-`Override` decision, because everything is
//! embedded the same way.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Serialize;

use crate::{Loader, LoaderKind, LoaderSpec};

#[derive(Debug, Serialize)]
pub struct PrismPack {
    pub components: Vec<Component>,
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
}

#[derive(Debug, Serialize)]
pub struct Component {
    pub uid: String,
    pub version: String,
}

/// Build the `components` list for `mmc-pack.json`.
///
/// Always emits `net.minecraft`. If `loader` is set, appends the loader's
/// component with Prism's UID (`net.minecraftforge` / `net.neoforged` /
/// `net.fabricmc.fabric-loader`).
///
/// Same reasoning as `mrpack::dependencies_from`: a channel spec
/// (`Latest`/`Recommended`) would drift on every re-export, so we
/// reject those loudly and force the user to pin.
pub fn components_from(minecraft: &str, loader: Option<&Loader>) -> anyhow::Result<Vec<Component>> {
    let mut components = vec![Component {
        uid: "net.minecraft".to_owned(),
        version: minecraft.to_owned(),
    }];
    if let Some(loader) = loader {
        let version = match &loader.spec {
            LoaderSpec::Pinned(v) => v.clone(),
            LoaderSpec::Latest | LoaderSpec::Recommended => bail!(
                "prism export requires a pinned loader version; got {:?}. \
                 Change `loader` in cart.toml to a specific version like \
                 `loader = {{ forge = \"47.2.0\" }}`.",
                loader.spec
            ),
        };
        let uid = match loader.kind {
            LoaderKind::Forge => "net.minecraftforge",
            LoaderKind::Fabric => "net.fabricmc.fabric-loader",
            LoaderKind::NeoForge => "net.neoforged",
        };
        components.push(Component {
            uid: uid.to_owned(),
            version,
        });
    }
    Ok(components)
}

/// Render `instance.cfg` for a Prism/MultiMC instance.
///
/// `InstanceType=OneSix` marks the modern instance format (MultiMC's
/// legacy name; Prism inherited it). Newlines inside `notes` are
/// encoded as literal `\n` — the config format is line-oriented, and
/// unescaped newlines would corrupt subsequent keys.
pub fn instance_cfg(name: &str, notes: Option<&str>) -> String {
    let mut s = String::from("InstanceType=OneSix\n");
    s.push_str("name=");
    s.push_str(name);
    s.push('\n');
    if let Some(notes) = notes {
        s.push_str("notes=");
        s.push_str(&notes.replace('\n', "\\n"));
        s.push('\n');
    }
    s
}

/// Write the Prism instance ZIP.
///
/// Every entry lives under `<instance_name>/`. `overrides` entries
/// arrive already fully-qualified (e.g. `"<name>/.minecraft/config/x.toml"`)
/// so the caller controls the exact in-archive layout of the overrides
/// mirror.
pub fn write_pack(
    instance_name: &str,
    pack: &PrismPack,
    cfg: &str,
    mods: &[(String, PathBuf)],
    overrides: &[(String, PathBuf)],
    output: &Path,
) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(output).with_context(|| format!("create {}", output.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let cfg_path = format!("{instance_name}/instance.cfg");
    writer
        .start_file(&cfg_path, options)
        .with_context(|| format!("start {cfg_path}"))?;
    writer
        .write_all(cfg.as_bytes())
        .with_context(|| format!("write {cfg_path}"))?;

    let pack_bytes = serde_json::to_vec_pretty(pack).context("serialize mmc-pack.json")?;
    let pack_path = format!("{instance_name}/mmc-pack.json");
    writer
        .start_file(&pack_path, options)
        .with_context(|| format!("start {pack_path}"))?;
    writer
        .write_all(&pack_bytes)
        .with_context(|| format!("write {pack_path}"))?;

    for (filename, source) in mods {
        let bytes =
            std::fs::read(source).with_context(|| format!("read mod jar {}", source.display()))?;
        let dest = format!("{instance_name}/.minecraft/mods/{filename}");
        writer
            .start_file(&dest, options)
            .with_context(|| format!("start ZIP entry {dest}"))?;
        writer
            .write_all(&bytes)
            .with_context(|| format!("write ZIP entry {dest}"))?;
    }

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

    writer.finish().context("finalize Prism instance archive")?;
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
    fn components_vanilla_has_only_minecraft() {
        let c = components_from("1.20.1", None).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].uid, "net.minecraft");
        assert_eq!(c[0].version, "1.20.1");
    }

    #[test]
    fn components_maps_forge_to_net_minecraftforge() {
        let c = components_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].uid, "net.minecraftforge");
        assert_eq!(c[1].version, "47.2.0");
    }

    /// Fabric's UID is `net.fabricmc.fabric-loader` — the whole
    /// dot-and-hyphen shape is easy to mistype; lock it in.
    #[test]
    fn components_maps_fabric_to_net_fabricmc_fabric_loader() {
        let c = components_from("1.20.1", Some(&fabric_pinned("0.15.7"))).unwrap();
        assert_eq!(c[1].uid, "net.fabricmc.fabric-loader");
        assert_eq!(c[1].version, "0.15.7");
    }

    #[test]
    fn components_maps_neoforge_to_net_neoforged() {
        let c = components_from("1.21.1", Some(&neoforge_pinned("21.1.242"))).unwrap();
        assert_eq!(c[1].uid, "net.neoforged");
        assert_eq!(c[1].version, "21.1.242");
    }

    #[test]
    fn components_rejects_latest_channel() {
        let loader = Loader::forge(LoaderSpec::Latest);
        let err = components_from("1.20.1", Some(&loader)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("pinned"), "expected pin hint: {msg}");
        assert!(msg.contains("cart.toml"), "expected cart.toml hint: {msg}");
    }

    #[test]
    fn components_rejects_recommended_channel() {
        let loader = Loader::forge(LoaderSpec::Recommended);
        let err = components_from("1.20.1", Some(&loader)).unwrap_err();
        assert!(err.to_string().contains("pinned"));
    }

    // ── instance.cfg ─────────────────────────────────────────────────

    #[test]
    fn instance_cfg_has_required_keys() {
        let cfg = instance_cfg("my-pack", None);
        assert!(cfg.contains("InstanceType=OneSix\n"), "cfg was: {cfg}");
        assert!(cfg.contains("name=my-pack\n"), "cfg was: {cfg}");
        assert!(!cfg.contains("notes="), "no notes expected: {cfg}");
    }

    #[test]
    fn instance_cfg_includes_notes_when_given() {
        let cfg = instance_cfg("my-pack", Some("single line"));
        assert!(cfg.contains("notes=single line\n"), "cfg was: {cfg}");
    }

    /// Multi-line notes must be encoded as literal `\n` — the config
    /// format is line-oriented, and a raw newline in the value would
    /// corrupt whatever key comes after it.
    #[test]
    fn instance_cfg_escapes_newlines_in_notes() {
        let cfg = instance_cfg("my-pack", Some("line1\nline2"));
        assert!(
            cfg.contains("notes=line1\\nline2\n"),
            "expected escaped \\n in cfg: {cfg}"
        );
        // Exactly one raw newline after the encoded notes value.
        let notes_line = cfg.lines().find(|l| l.starts_with("notes=")).unwrap();
        assert_eq!(notes_line, "notes=line1\\nline2");
    }

    // ── mmc-pack.json layout ─────────────────────────────────────────

    /// Golden-JSON test: hand-build a small `PrismPack`, serialize it,
    /// and compare against a checked-in fixture. Locks in `formatVersion`
    /// field spelling and the `components[].{uid,version}` layout. A
    /// drift here means the emitted `mmc-pack.json` stops loading in
    /// Prism.
    #[test]
    fn hand_built_pack_matches_golden() {
        let pack = PrismPack {
            components: components_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap(),
            format_version: 1,
        };

        let actual = serde_json::to_string_pretty(&pack).unwrap();
        let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/prism/mmc_pack_minimal.json");
        let expected = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", fixture_path.display()));

        assert_eq!(actual.trim_end(), expected.trim_end());
    }

    // ── ZIP writing ──────────────────────────────────────────────────

    /// Round-trip: write a small instance (cfg + pack + one mod + one
    /// override), reopen it, and assert everything landed at the right
    /// paths with unchanged bytes. Locks the archive layout — the
    /// `<name>/` outer folder, `.minecraft/mods/` for jars,
    /// `.minecraft/` mirror for overrides — which is what makes the
    /// archive a Prism instance and not just a random ZIP.
    #[test]
    fn write_pack_round_trip() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("foo.jar");
        std::fs::write(&jar, b"jar-bytes").unwrap();
        let cfg_source = dir.path().join("config.toml");
        std::fs::write(&cfg_source, b"hello=world\n").unwrap();

        let pack = PrismPack {
            components: components_from("1.20.1", Some(&forge_pinned("47.2.0"))).unwrap(),
            format_version: 1,
        };
        let cfg = instance_cfg("my-pack", Some("smoke test"));
        let mods = vec![("foo.jar".to_owned(), jar)];
        let overrides = vec![(
            "my-pack/.minecraft/config/config.toml".to_owned(),
            cfg_source,
        )];
        let output = dir.path().join("out.zip");

        write_pack("my-pack", &pack, &cfg, &mods, &overrides, &output).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        let mut cfg_bytes = Vec::new();
        archive
            .by_name("my-pack/instance.cfg")
            .unwrap()
            .read_to_end(&mut cfg_bytes)
            .unwrap();
        assert_eq!(std::str::from_utf8(&cfg_bytes).unwrap(), cfg);

        let mut pack_bytes = Vec::new();
        archive
            .by_name("my-pack/mmc-pack.json")
            .unwrap()
            .read_to_end(&mut pack_bytes)
            .unwrap();
        let round_tripped: serde_json::Value = serde_json::from_slice(&pack_bytes).unwrap();
        assert_eq!(round_tripped["formatVersion"], 1);
        assert_eq!(round_tripped["components"][0]["uid"], "net.minecraft");
        assert_eq!(round_tripped["components"][1]["uid"], "net.minecraftforge");

        let mut jar_bytes = Vec::new();
        archive
            .by_name("my-pack/.minecraft/mods/foo.jar")
            .unwrap()
            .read_to_end(&mut jar_bytes)
            .unwrap();
        assert_eq!(jar_bytes, b"jar-bytes");

        let mut cfg_toml = Vec::new();
        archive
            .by_name("my-pack/.minecraft/config/config.toml")
            .unwrap()
            .read_to_end(&mut cfg_toml)
            .unwrap();
        assert_eq!(cfg_toml, b"hello=world\n");
    }
}
