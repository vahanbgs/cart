//! Shared pack-scaffolding logic used by both `cart init` and `cart new`.
//!
//! Both commands, once their preconditions are satisfied, do the same
//! work: derive a pack name from the directory, prompt for the Minecraft
//! version (unless `--mv` was passed) and mod loader, and write out a
//! fresh `cart.toml`. This module owns that flow; the two command files
//! are thin wrappers that enforce their respective preconditions before
//! calling in.
//!
//! Contract: `scaffold(cli, dir)` assumes `dir` exists and does not
//! contain a `cart.toml`. Callers enforce that.

use std::{
    fmt::{self, Display},
    path::Path,
};

use anyhow::Context;
use cart::api::{
    Endpoint,
    fabric::GameVersions,
    forge::ForgePromotions,
    neoforge::MavenMetadata,
    piston::{Kind, VersionManifest},
};
use inquire::Select;
use reqwest::Client;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::manifest;

use super::Cli;

/// Menu options for the loader picker. Ordered Fabric → NeoForge → Forge →
/// Vanilla: the arrow keys default to the first entry and someone reaching
/// for cart is likely wanting a modded setup; Fabric on top is the most
/// common modern choice, NeoForge second for modern Forge-family, plain
/// Forge for legacy MC versions.
#[derive(Clone, Copy)]
enum LoaderChoice {
    Fabric,
    NeoForge,
    Forge,
    Vanilla,
}

impl Display for LoaderChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fabric => "Fabric   —  modern, lightweight",
            Self::NeoForge => "NeoForge —  modern Forge fork (1.20.2+)",
            Self::Forge => "Forge    —  classic, mature",
            Self::Vanilla => "Vanilla  —  no mod loader",
        })
    }
}

impl LoaderChoice {
    /// The bare-string sugar we write into `cart.toml`. `None` for vanilla
    /// (the `loader` key is omitted entirely). All loader variants default
    /// to `latest` — matches the `loader = "fabric"` etc. sugar the manifest
    /// parser recognises.
    fn toml_value(self) -> Option<&'static str> {
        match self {
            Self::Fabric => Some("fabric"),
            Self::NeoForge => Some("neoforge"),
            Self::Forge => Some("forge"),
            Self::Vanilla => None,
        }
    }
}

pub(super) async fn scaffold(cli: &Cli, dir: &Path) -> anyhow::Result<()> {
    let name = derive_pack_name(dir)?;

    // If the user passed `--mv` they've already made the MC-version
    // decision on the command line — skip that prompt. The loader
    // still needs picking.
    let mc_version = match cli.minecraft_version.as_deref() {
        Some(v) => v.to_owned(),
        None => pick_minecraft_version().await?,
    };

    let loader = pick_loader(&mc_version).await?;

    let mut document = DocumentMut::new();
    // Order matters — cart.toml reads top-to-bottom in the parse
    // tests (`parses_manifest_with_all_pack_fields`) and Cargo-style
    // metadata belongs at the top of the file.
    document["name"] = value(name);
    document["version"] = value("0.1.0");
    document["minecraft"] = value(mc_version);
    if let Some(loader_str) = loader.toml_value() {
        document["loader"] = value(loader_str);
    }
    let mut mods = Table::new();
    mods.set_implicit(false);
    document["mods"] = Item::Table(mods);

    manifest::save_document(&dir.join("cart.toml"), &document).await?;

    Ok(())
}

/// Pick a pack name from the directory the manifest lives in. Callers
/// hand in the raw CLI path; relative paths, `.`, and trailing slashes
/// all get resolved by canonicalize so the last component reflects what
/// the user actually sees in `ls`.
///
/// Requires the directory to exist — callers must ensure that (either
/// by creating it, in `cart new`, or by enforcing it as a precondition,
/// in `cart init`). Returns an error if the resolved path has no file
/// component (e.g. `cart init /`) or a non-UTF-8 name.
fn derive_pack_name(path: &Path) -> anyhow::Result<String> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))?;
    canonical
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .with_context(|| {
            format!(
                "cannot derive pack name from {} — no file component or not UTF-8",
                canonical.display()
            )
        })
}

/// Fetch the Piston release list and open an inquire Select. Releases are
/// listed newest-first by Piston, so the arrow-key default is the latest
/// release — the "just press enter" path picks a sensible modern version.
/// Snapshots/old-alpha/old-beta are filtered out; hand-editing `cart.toml`
/// covers the rare pinning-to-a-snapshot case.
async fn pick_minecraft_version() -> anyhow::Result<String> {
    let manifest: VersionManifest = Client::new()
        .get(VersionManifest::url().clone())
        .send()
        .await
        .context("failed to fetch Minecraft version manifest")?
        .error_for_status()?
        .json()
        .await
        .context("failed to parse Minecraft version manifest")?;

    let releases: Vec<String> = manifest
        .versions()
        .iter()
        .filter(|v| matches!(v.kind, Kind::Release))
        .map(|v| v.id.clone())
        .collect();

    // inquire is blocking; run on a dedicated thread so it doesn't stall
    // other tokio tasks. There aren't any right now, but the pattern is
    // cheap and keeps this reusable if scaffolding grows more concurrent
    // work.
    Ok(tokio::task::spawn_blocking(move || {
        Select::new("Minecraft version", releases)
            .with_page_size(12)
            .with_help_message("↑↓ navigate • type to filter • enter to select")
            .prompt()
    })
    .await??)
}

/// Loader menu, filtered by what actually supports `mc_version`. Fabric
/// only offers 1.14+ (whatever appears in `/v2/versions/game`); Forge
/// only offers MC versions that have a promotions entry — so we don't
/// suggest an impossible combination the launcher would fail on.
///
/// If a support check errors (e.g. offline), the loader is *hidden*.
/// Hiding beats silently allowing a combination that `cart run` can't
/// resolve, and Vanilla is always available as an escape hatch.
async fn pick_loader(mc_version: &str) -> anyhow::Result<LoaderChoice> {
    let http = Client::new();
    let (fabric_ok, neoforge_ok, forge_ok) = tokio::join!(
        fabric_supports_mc(&http, mc_version),
        neoforge_supports_mc(&http, mc_version),
        forge_supports_mc(&http, mc_version),
    );

    let mut options = Vec::new();
    if fabric_ok {
        options.push(LoaderChoice::Fabric);
    }
    if neoforge_ok {
        options.push(LoaderChoice::NeoForge);
    }
    if forge_ok {
        options.push(LoaderChoice::Forge);
    }
    options.push(LoaderChoice::Vanilla);

    Ok(tokio::task::spawn_blocking(move || {
        Select::new("Mod loader", options)
            .with_help_message("↑↓ navigate • enter to select")
            .prompt()
    })
    .await??)
}

async fn fabric_supports_mc(http: &Client, mc_version: &str) -> bool {
    async fn inner(http: &Client, mc_version: &str) -> anyhow::Result<bool> {
        let list: GameVersions = http
            .get(GameVersions::url().clone())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(list.contains(mc_version))
    }
    inner(http, mc_version).await.unwrap_or(false)
}

async fn forge_supports_mc(http: &Client, mc_version: &str) -> bool {
    async fn inner(http: &Client, mc_version: &str) -> anyhow::Result<bool> {
        let promotions: ForgePromotions = http
            .get(ForgePromotions::url().clone())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(promotions.supports_mc(mc_version))
    }
    inner(http, mc_version).await.unwrap_or(false)
}

async fn neoforge_supports_mc(http: &Client, mc_version: &str) -> bool {
    MavenMetadata::fetch(http)
        .await
        .map(|m| m.supports_mc(mc_version))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_pack_name_returns_final_component() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("my-pack");
        std::fs::create_dir(&pack_dir).unwrap();

        assert_eq!(derive_pack_name(&pack_dir).unwrap(), "my-pack");
    }

    /// Trailing slashes, `.` segments, and relative paths all get
    /// normalized by `canonicalize` — the result is the actual name
    /// the user sees in `ls`, not whatever they happened to type.
    #[test]
    fn derive_pack_name_normalizes_trailing_slash_and_dot() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("weird-pack");
        std::fs::create_dir(&pack_dir).unwrap();

        // e.g. `weird-pack/./`
        let with_dot = pack_dir.join(".");
        assert_eq!(derive_pack_name(&with_dot).unwrap(), "weird-pack");
    }

    /// Missing directory is a real error (`canonicalize` needs it to
    /// exist) rather than a silent fallback — surface it so the caller
    /// notices the required precondition wasn't met.
    #[test]
    fn derive_pack_name_errors_on_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(derive_pack_name(&missing).is_err());
    }
}
