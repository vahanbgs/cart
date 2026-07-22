use cart::api::modrinth;
use clap::{Args, Subcommand};
use reqwest::Client;

use crate::{config::Config, manifest};

use super::Cli;

#[derive(Args)]
pub struct Modrinth {
    #[command(subcommand)]
    pub command: ModrinthCommand,
}

#[derive(Subcommand)]
pub enum ModrinthCommand {
    Add(Add),
}

impl Modrinth {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            ModrinthCommand::Add(add) => add.run(cli).await,
        }
    }
}

#[derive(Args)]
pub struct Add {
    /// Modrinth project slug.
    pub slug: String,

    /// Pin to a specific Modrinth `version_number` string. Default:
    /// newest version compatible with the manifest's Minecraft version
    /// and loader.
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
        let loader = match config.manifest().loader.as_ref().map(|l| l.kind) {
            Some(cart::LoaderKind::Fabric) => "fabric",
            Some(cart::LoaderKind::Forge) => "forge",
            Some(cart::LoaderKind::NeoForge) => "neoforge",
            None => "vanilla",
        };

        let http = Client::new();
        let resolved = modrinth::resolve(
            &http,
            &self.slug,
            self.version.as_deref(),
            minecraft_version,
            loader,
        )
        .await?;

        for dependency in &resolved.dependencies {
            if dependency.dependency_type == modrinth::DependencyType::Required {
                let id = dependency.project_id.as_deref().unwrap_or("<unknown>");
                tracing::warn!(
                    "required dependency (modrinth project id {id}) — add it explicitly if not already present"
                );
            }
        }

        let manifest_key = self.name.as_deref().unwrap_or(&resolved.project_slug);

        let mut document = manifest::load_document(&path).await?;
        manifest::add_modrinth_mod(
            &mut document,
            manifest_key,
            &resolved.project_slug,
            &resolved.version_number,
            self.disabled,
        )?;
        manifest::save_document(&path, &document).await?;

        tracing::info!(
            "added {manifest_key} ({title} {version_number}, {filename})",
            title = resolved.project_title,
            version_number = resolved.version_number,
            filename = resolved.file.filename,
        );

        Ok(())
    }
}
