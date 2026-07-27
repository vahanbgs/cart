use cart::api::modrinth;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Modrinth, ModrinthCommand,
    args::modrinth::{Add, Find, Search},
    hit_view,
};

impl Modrinth {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            ModrinthCommand::Add(add) => add.run(cli).await,
            ModrinthCommand::Search(search) => search.run(cli).await,
            ModrinthCommand::Find(find) => find.run(cli).await,
        }
    }
}

/// Loader string as Modrinth expects for `/v2/project/{slug}/version`
/// resolution — vanilla manifests become the literal `"vanilla"` since
/// that's Modrinth's convention for no-loader compatibility.
fn resolve_loader(config: &Config<'_>) -> &'static str {
    config
        .manifest()
        .loader
        .as_ref()
        .map(|l| l.kind.as_modrinth())
        .unwrap_or("vanilla")
}

/// Loader facet for Modrinth's `/v2/search`. Unlike resolution there is
/// no `"vanilla"` category, so a no-loader manifest maps to `None` and
/// callers drop the facet entirely.
fn search_loader(config: &Config<'_>) -> Option<&'static str> {
    config
        .manifest()
        .loader
        .as_ref()
        .map(|l| l.kind.as_modrinth())
}

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let minecraft_version = config.minecraft_version();
        let loader = resolve_loader(&config);

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

impl Search {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Load the manifest for its Minecraft version + loader — the
        // whole point of `search` is to only surface installable hits,
        // so a missing cart.toml is a hard error, same as `find`.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = search_loader(&config);

        let http = Client::new();
        let hits =
            modrinth::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            return Ok(());
        }

        let rows: Vec<hit_view::HitRow> = hits.iter().map(Into::into).collect();
        hit_view::print_search_results(&rows);

        Ok(())
    }
}

impl Find {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Fail fast if there's no manifest to add to — better than
        // spending a network round-trip and a picker interaction
        // before finding out.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = search_loader(&config);

        let http = Client::new();
        let hits =
            modrinth::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            tracing::info!("no results for '{}'", self.query);
            return Ok(());
        }

        let rows: Vec<hit_view::HitRow> = hits.iter().map(Into::into).collect();
        let page_size = self.limit.min(15) as usize;
        let picked = hit_view::pick_hit(rows, "Add which mod?", page_size).await?;

        let add = Add {
            slug: picked.slug,
            version: None,
            name: None,
            disabled: self.disabled,
        };
        add.run(cli).await
    }
}
