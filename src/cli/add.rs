use anyhow::Context;
use cart::api::{curseforge, modrinth};
use clap::Args;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::Cli;

/// Env var holding the CurseForge API key. Required only for
/// `cart add curseforge:<slug>` — Modrinth adds don't touch it.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

#[derive(Args)]
pub struct Add {
    /// Project slug. Defaults to Modrinth; prefix with `curseforge:` to
    /// pull from CurseForge (e.g. `curseforge:jei`). Modrinth/CurseForge
    /// slugs never contain `:`, so the split is unambiguous.
    pub slug: String,

    /// Pin to a specific version. On Modrinth this is a `version_number`
    /// string; on CurseForge it's a numeric file id. Default: newest
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

        if let Some(slug) = self.slug.strip_prefix("curseforge:") {
            self.add_curseforge(&config, &path, slug).await
        } else {
            self.add_modrinth(&config, &path, &self.slug).await
        }
    }

    async fn add_modrinth(
        &self,
        config: &Config<'_>,
        path: &std::path::Path,
        slug: &str,
    ) -> anyhow::Result<()> {
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
            slug,
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

        let mut document = manifest::load_document(path).await?;
        manifest::add_modrinth_mod(
            &mut document,
            manifest_key,
            &resolved.project_slug,
            &resolved.version_number,
            self.disabled,
        )?;
        manifest::save_document(path, &document).await?;

        tracing::info!(
            "added {manifest_key} ({title} {version_number}, {filename})",
            title = resolved.project_title,
            version_number = resolved.version_number,
            filename = resolved.file.filename,
        );

        Ok(())
    }

    async fn add_curseforge(
        &self,
        config: &Config<'_>,
        path: &std::path::Path,
        slug: &str,
    ) -> anyhow::Result<()> {
        let minecraft_version = &config.manifest().minecraft;
        let loader = config.manifest().loader.as_ref().map(|l| match l.kind {
            cart::LoaderKind::Fabric => curseforge::LoaderType::Fabric,
            cart::LoaderKind::Forge => curseforge::LoaderType::Forge,
            cart::LoaderKind::NeoForge => curseforge::LoaderType::NeoForge,
        });

        let key = std::env::var(CURSEFORGE_API_KEY_ENV).with_context(|| {
            format!(
                "cart add curseforge:<slug> needs {CURSEFORGE_API_KEY_ENV} — get a key at \
                 https://console.curseforge.com/ and export it"
            )
        })?;
        let http = curseforge::client(&key)?;

        let project = curseforge::find_project_by_slug(&http, slug).await?;

        // Pin: --version parses as a numeric file id on CF; loose picks
        // newest file compatible with the manifest's mc + loader.
        let file = if let Some(pin) = self.version.as_deref() {
            let file_id: u32 = pin.parse().with_context(|| {
                format!(
                    "curseforge --version must be a numeric file id (got '{pin}')"
                )
            })?;
            curseforge::fetch_file(&http, project.id, file_id).await?
        } else {
            curseforge::latest_file(&http, project.id, minecraft_version, loader).await?
        };

        let manifest_key = self.name.as_deref().unwrap_or(&project.slug);

        let mut document = manifest::load_document(path).await?;
        manifest::add_curseforge_mod(
            &mut document,
            manifest_key,
            project.id,
            file.id,
            self.disabled,
        )?;
        manifest::save_document(path, &document).await?;

        tracing::info!(
            "added {manifest_key} ({name} file {file_id}, {filename})",
            name = project.name,
            file_id = file.id,
            filename = file.file_name,
        );

        Ok(())
    }
}
