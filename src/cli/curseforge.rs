use anyhow::Context;
use cart::api::curseforge;
use clap::{Args, Subcommand};
use reqwest::Client;

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
    Search(Search),
}

impl Curseforge {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            CurseforgeCommand::Add(add) => add.run(cli).await,
            CurseforgeCommand::Search(search) => search.run(cli).await,
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

#[derive(Args)]
pub struct Search {
    /// Free-text query. Passed verbatim to CurseForge's `/v1/mods/search`.
    pub query: String,

    /// Maximum number of results to print. CurseForge's `pageSize`
    /// cap is 50; cart doesn't paginate.
    #[arg(long, default_value_t = 10)]
    pub limit: u32,
}

impl Search {
    pub async fn run(&self, _cli: &Cli) -> anyhow::Result<()> {
        let http = cf_client()?;
        let hits = curseforge::search(&http, &self.query, self.limit).await?;

        if hits.is_empty() {
            return Ok(());
        }

        let slug_width = hits.iter().map(|h| h.slug.len()).max().unwrap_or(0);
        let name_width = hits.iter().map(|h| h.name.len()).max().unwrap_or(0);
        let downloads_labels: Vec<String> = hits
            .iter()
            .map(|h| format_downloads(h.download_count))
            .collect();
        let downloads_width = downloads_labels
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        for (hit, downloads) in hits.iter().zip(downloads_labels.iter()) {
            let summary = truncate(&hit.summary, 60);
            println!(
                "{slug:<slug_width$}  {name:<name_width$}  {downloads:>downloads_width$}  {summary}",
                slug = hit.slug,
                name = hit.name,
            );
        }

        Ok(())
    }
}

/// Compact download counts: `77.0M`, `31.8k`, `310`. Same shape as
/// `mr search` so both trees read the same at a glance.
fn format_downloads(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Truncate on character boundaries — CurseForge summaries can contain
/// multi-byte characters and a byte-based slice would panic.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
