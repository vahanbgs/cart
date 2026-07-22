use std::fmt::{self, Display, Formatter};

use cart::api::modrinth;
use inquire::Select;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Modrinth, ModrinthCommand,
    args::modrinth::{Add, Find, Search},
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

impl Search {
    pub async fn run(&self, _cli: &Cli) -> anyhow::Result<()> {
        let http = Client::new();
        let hits = modrinth::search(&http, &self.query, self.limit).await?;

        if hits.is_empty() {
            return Ok(());
        }

        let slug_width = hits.iter().map(|h| h.slug.len()).max().unwrap_or(0);
        let title_width = hits.iter().map(|h| h.title.len()).max().unwrap_or(0);
        let downloads_labels: Vec<String> =
            hits.iter().map(|h| format_downloads(h.downloads)).collect();
        let downloads_width = downloads_labels
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        for (hit, downloads) in hits.iter().zip(downloads_labels.iter()) {
            let description = truncate(&hit.description, 60);
            println!(
                "{slug:<slug_width$}  {title:<title_width$}  {downloads:>downloads_width$}  {description}",
                slug = hit.slug,
                title = hit.title,
            );
        }

        Ok(())
    }
}

impl Find {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Fail fast if there's no manifest to add to — better than
        // spending a network round-trip and a picker interaction
        // before finding out.
        Config::load(cli).await?;

        let http = Client::new();
        let hits = modrinth::search(&http, &self.query, self.limit).await?;

        if hits.is_empty() {
            tracing::info!("no results for '{}'", self.query);
            return Ok(());
        }

        let choices = HitChoice::from_hits(hits);

        // inquire is blocking; keep it off the tokio runtime.
        let page_size = self.limit.min(15) as usize;
        let picked = tokio::task::spawn_blocking(move || {
            Select::new("Add which mod?", choices)
                .with_page_size(page_size)
                .with_help_message("↑↓ navigate • type to filter • enter to select")
                .prompt()
        })
        .await??;

        let add = Add {
            slug: picked.hit.slug.clone(),
            version: None,
            name: None,
            disabled: self.disabled,
        };
        add.run(cli).await
    }
}

/// Wraps a `SearchHit` with a pre-computed aligned Display label. inquire
/// filters by the Display output, so pre-aligning it means users can type
/// to narrow against slug/title/author in one string without inquire
/// re-formatting on every keystroke.
struct HitChoice {
    hit: modrinth::SearchHit,
    label: String,
}

impl HitChoice {
    fn from_hits(hits: Vec<modrinth::SearchHit>) -> Vec<Self> {
        let slug_width = hits.iter().map(|h| h.slug.len()).max().unwrap_or(0);
        let title_width = hits.iter().map(|h| h.title.len()).max().unwrap_or(0);
        let downloads_labels: Vec<String> =
            hits.iter().map(|h| format_downloads(h.downloads)).collect();
        let downloads_width = downloads_labels
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        hits.into_iter()
            .zip(downloads_labels)
            .map(|(hit, downloads)| {
                let description = truncate(&hit.description, 60);
                let label = format!(
                    "{slug:<slug_width$}  {title:<title_width$}  {downloads:>downloads_width$}  {description}",
                    slug = hit.slug,
                    title = hit.title,
                );
                HitChoice { hit, label }
            })
            .collect()
    }
}

impl Display for HitChoice {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Compact download counts for the search table: `77.0M`, `31.8k`, `310`.
/// Bare `77041094` is noise in a narrow column and Modrinth's own site
/// uses the same abbreviation, so users read it fluently.
fn format_downloads(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Truncate on character boundaries — Modrinth descriptions can contain
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
