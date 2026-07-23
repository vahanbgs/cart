use anyhow::Context;
use cart::api::curseforge;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Curseforge, CurseforgeCommand,
    args::curseforge::{Add, Find, Search},
    hit_view,
};

/// Env var holding the CurseForge API key. Only read when a CurseForge
/// subcommand actually runs — the Modrinth tree never touches it.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

impl Curseforge {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            CurseforgeCommand::Add(add) => add.run(cli).await,
            CurseforgeCommand::Search(search) => search.run(cli).await,
            CurseforgeCommand::Find(find) => find.run(cli).await,
        }
    }
}

/// `CURSEFORGE_API_KEY` guarded read used by every CF-hitting subcommand.
/// Same error text everywhere so users see one consistent hint.
fn cf_client() -> anyhow::Result<Client> {
    let key = std::env::var(CURSEFORGE_API_KEY_ENV).with_context(|| {
        format!(
            "cart curseforge subcommands need {CURSEFORGE_API_KEY_ENV} — get a key at \
             https://console.curseforge.com/ and export it"
        )
    })?;
    curseforge::client(&key)
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

        let http = cf_client()?;

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

impl Search {
    pub async fn run(&self, _cli: &Cli) -> anyhow::Result<()> {
        let http = cf_client()?;
        let hits = curseforge::search(&http, &self.query, self.limit).await?;

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
        // Fail fast on a missing manifest before touching the network
        // or the picker.
        Config::load(cli).await?;

        let http = cf_client()?;
        let hits = curseforge::search(&http, &self.query, self.limit).await?;

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
