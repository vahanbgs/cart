use anyhow::{Context, bail};
use cart::api::modrinth;
use clap::Args;
use reqwest::{Client, StatusCode};

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

        // Validate the slug exists — a 404 here means the user typo'd.
        let project_response = http
            .get(modrinth::project_url(&self.slug))
            .send()
            .await?;
        if project_response.status() == StatusCode::NOT_FOUND {
            bail!("slug '{}' not found on Modrinth", self.slug);
        }
        let project: modrinth::Project = project_response.error_for_status()?.json().await?;

        // Fetch the pre-filtered version list.
        let versions: Vec<modrinth::Version> = http
            .get(modrinth::versions_url(
                &self.slug,
                minecraft_version,
                loader,
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let version = match &self.version {
            Some(pin) => versions
                .into_iter()
                .find(|v| v.version_number == *pin)
                .with_context(|| {
                    format!(
                        "no version '{pin}' of '{}' compatible with minecraft {minecraft_version} + {loader}",
                        project.slug
                    )
                })?,
            None => versions
                .into_iter()
                .max_by_key(|v| v.date_published)
                .with_context(|| {
                    format!(
                        "no version of '{}' compatible with minecraft {minecraft_version} + {loader}",
                        project.slug
                    )
                })?,
        };

        let file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or_else(|| version.files.first())
            .with_context(|| {
                format!(
                    "modrinth version {} has no files",
                    version.version_number
                )
            })?;

        for dependency in &version.dependencies {
            if dependency.dependency_type == modrinth::DependencyType::Required {
                let id = dependency.project_id.as_deref().unwrap_or("<unknown>");
                tracing::warn!(
                    "required dependency (modrinth project id {id}) — add it explicitly if not already present"
                );
            }
        }

        let manifest_key = self.name.as_deref().unwrap_or(&project.slug);

        let mut document = manifest::load_document(&path).await?;
        manifest::add_mod(&mut document, manifest_key, &file.url, self.disabled)?;
        manifest::save_document(&path, &document).await?;

        tracing::info!(
            "added {manifest_key} ({title} {version_number}, {filename})",
            title = project.title,
            version_number = version.version_number,
            filename = file.filename,
        );

        Ok(())
    }
}
