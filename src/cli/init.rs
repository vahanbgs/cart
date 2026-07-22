use std::{
    fmt::{self, Display},
    path::PathBuf,
};

use anyhow::Context;
use cart::api::{
    Endpoint,
    fabric::GameVersions,
    forge::ForgePromotions,
    neoforge::MavenMetadata,
    piston::{Kind, VersionManifest},
};
use clap::Args;
use inquire::Select;
use reqwest::Client;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::manifest;

use super::Cli;

#[derive(Args)]
pub struct Init {
    pub path: PathBuf,
}

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

impl Init {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        fs::create_dir_all(&self.path).await?;

        // If the user passed `--mv` they've already made the MC-version
        // decision on the command line — skip that prompt. The loader
        // still needs picking.
        let mc_version = match cli.minecraft_version.as_deref() {
            Some(v) => v.to_owned(),
            None => pick_minecraft_version().await?,
        };

        let loader = pick_loader(&mc_version).await?;

        let mut document = DocumentMut::new();
        document["minecraft"] = value(mc_version);
        if let Some(loader_str) = loader.toml_value() {
            document["loader"] = value(loader_str);
        }
        let mut mods = Table::new();
        mods.set_implicit(false);
        document["mods"] = Item::Table(mods);

        manifest::save_document(&self.path.join("cart.toml"), &document).await?;

        Ok(())
    }
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
    // cheap and keeps this reusable if init grows more concurrent work.
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
