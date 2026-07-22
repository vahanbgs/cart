use anyhow::Context;
use cart::api::curseforge;
use clap::{Args, Subcommand};

use crate::{config::Config, manifest};

use super::Cli;

/// Env var holding the CurseForge API key. Only read when a CurseForge
/// subcommand actually runs — the Modrinth tree never touches it.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

#[derive(Args)]
pub struct Curseforge {
    #[command(subcommand)]
    pub command: CurseforgeCommand,
}

#[derive(Subcommand)]
pub enum CurseforgeCommand {
    Add(Add),
}

impl Curseforge {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            CurseforgeCommand::Add(add) => add.run(cli).await,
        }
    }
}

#[derive(Args)]
pub struct Add {
    /// CurseForge project slug.
    pub slug: String,

    /// Pin to a specific numeric CurseForge file id. Default: newest
    /// file compatible with the manifest's Minecraft version and loader.
    #[arg(long)]
    pub version: Option<String>,

    /// Key to use under `[mods]` in `cart.toml`. Default: the slug.
    #[arg(long)]
    pub name: Option<String>,

    /// Add the mod already disabled (placed as `<name>.jar.disabled`).
    #[arg(long)]
    pub disabled: bool,
}

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let minecraft_version = &config.manifest().minecraft;
        let loader = config.manifest().loader.as_ref().map(|l| match l.kind {
            cart::LoaderKind::Fabric => curseforge::LoaderType::Fabric,
            cart::LoaderKind::Forge => curseforge::LoaderType::Forge,
            cart::LoaderKind::NeoForge => curseforge::LoaderType::NeoForge,
        });

        let key = std::env::var(CURSEFORGE_API_KEY_ENV).with_context(|| {
            format!(
                "cart curseforge add needs {CURSEFORGE_API_KEY_ENV} — get a key at \
                 https://console.curseforge.com/ and export it"
            )
        })?;
        let http = curseforge::client(&key)?;

        let project = curseforge::find_project_by_slug(&http, &self.slug).await?;

        let file = if let Some(pin) = self.version.as_deref() {
            let file_id: u32 = pin.parse().with_context(|| {
                format!("curseforge --version must be a numeric file id (got '{pin}')")
            })?;
            curseforge::fetch_file(&http, project.id, file_id).await?
        } else {
            curseforge::latest_file(&http, project.id, minecraft_version, loader).await?
        };

        let manifest_key = self.name.as_deref().unwrap_or(&project.slug);

        let mut document = manifest::load_document(&path).await?;
        manifest::add_curseforge_mod(
            &mut document,
            manifest_key,
            project.id,
            file.id,
            self.disabled,
        )?;
        manifest::save_document(&path, &document).await?;

        tracing::info!(
            "added {manifest_key} ({name} file {file_id}, {filename})",
            name = project.name,
            file_id = file.id,
            filename = file.file_name,
        );

        Ok(())
    }
}
