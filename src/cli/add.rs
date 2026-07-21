use cart::api::modrinth;
use clap::Args;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::Cli;

#[derive(Args)]
pub struct Add {
    /// Modrinth project slug (e.g. `jei`, `appleskin`).
    pub slug: String,

    /// Pin to a specific `version_number` from Modrinth. Default: newest
    /// version compatible with the manifest's Minecraft version + loader.
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
        // TODO: expand when the manifest grows Fabric/NeoForge fields.
        let loader = if config.manifest().forge.is_some() {
            "forge"
        } else {
            "vanilla"
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
