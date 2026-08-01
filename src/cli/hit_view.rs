use std::fmt::{self, Display, Formatter};
use std::io::IsTerminal;
use std::path::PathBuf;

use cart::api::{curseforge, modrinth};
use futures::stream::StreamExt;
use inquire::Select;
use url::Url;

use crate::cli::icon_cache::IconCache;

/// Backend-agnostic search hit. Both Modrinth and CurseForge search
/// responses collapse to this shape via the `From` impls below — every
/// field-name difference (`title`/`name`, `description`/`summary`,
/// `downloads`/`download_count`) is bridged in one place so the renderer
/// stays backend-blind.
pub struct HitRow {
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub downloads: u64,
    /// The mod's icon on the backend's CDN. `None` when the project has
    /// no icon uploaded, or when a future schema drift makes the field
    /// undecodable — treated as "just render without an icon."
    pub icon_url: Option<Url>,
}

impl From<&modrinth::SearchHit> for HitRow {
    fn from(h: &modrinth::SearchHit) -> Self {
        HitRow {
            slug: h.slug.clone(),
            title: h.title.clone(),
            summary: h.description.clone(),
            downloads: h.downloads,
            icon_url: h.icon_url.clone(),
        }
    }
}

impl From<&curseforge::SearchHit> for HitRow {
    fn from(h: &curseforge::SearchHit) -> Self {
        HitRow {
            slug: h.slug.clone(),
            title: h.name.clone(),
            summary: h.summary.clone(),
            downloads: h.download_count,
            icon_url: h.logo_url.clone(),
        }
    }
}

/// Cap for the second-line summary. The old single-line layout capped at
/// 60 to leave room for slug/name/downloads on the same row; with the
/// summary on its own line we can spend more of the terminal width on it
/// without pushing the primary label off-screen.
const SUMMARY_MAX_CHARS: usize = 80;

/// Render one hit as a two-line block:
///
/// ```text
/// <title> · <slug> · <downloads>
///   <dim(summary)>
/// ```
///
/// No column padding — each entry sizes itself, so a long title on one
/// row doesn't pull short titles into empty space. If `summary` is
/// empty we skip line 2 rather than emit a dangling indent.
fn render_hit(row: &HitRow) -> String {
    let header = format!(
        "{title} · {slug} · {downloads}",
        title = row.title,
        slug = row.slug,
        downloads = format_downloads(row.downloads),
    );
    if row.summary.is_empty() {
        header
    } else {
        let summary = truncate(&row.summary, SUMMARY_MAX_CHARS);
        format!("{header}\n  {}", dim(&summary))
    }
}

/// Wrap `s` in an ANSI "dim" (faint) SGR pair when stdout is a real
/// terminal, otherwise return it verbatim. Kept manual to avoid pulling
/// in a color crate for one attribute — inquire's own transitive
/// `console` isn't usable without adding it to `Cargo.toml`.
fn dim(s: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_owned()
    }
}

/// One-line dim note printed above results when the search/find filter
/// dropped hits that are already declared in `cart.toml`. Shared between
/// the Modrinth and CurseForge subcommands so the wording stays in one
/// place.
pub fn print_hidden_note(hidden: usize, total: usize) {
    println!(
        "{}",
        dim(&format!(
            "{hidden} of {total} hidden (already in cart.toml)"
        ))
    );
}

/// Icon width in terminal cells, chosen to line up with the 2-line text
/// height for a compact side-by-side layout. Kept small on purpose:
/// bigger icons crowd the summary line and, on the half-block fallback,
/// still look pixelated at any size.
const ICON_CELLS_WIDE: u32 = 4;
const ICON_CELLS_TALL: u32 = 2;
/// One-cell padding between the icon and the text column.
const ICON_TEXT_GAP: u16 = 1;

/// Non-interactive stdout output for `mr search` / `cf search`.
///
/// When stdout is a TTY, prefetches icons via `cache` and prints each
/// hit as `<icon>  <header>\n         <summary>\n\n`, using viuer's
/// auto-detected terminal graphics protocol. When stdout is piped or an
/// icon can't be fetched, falls back to the text-only layout.
pub async fn print_search_results(rows: &[HitRow], cache: &IconCache) {
    let render_icons = std::io::stdout().is_terminal();
    let icons: Vec<Option<PathBuf>> = if render_icons {
        prefetch_icons(cache, rows).await
    } else {
        vec![None; rows.len()]
    };

    let mut first = true;
    for (row, icon) in rows.iter().zip(icons) {
        if !first {
            println!();
        }
        first = false;
        print_hit_with_icon(row, icon.as_deref());
    }
}

/// Prefetch each row's icon concurrently (bounded to 8 in-flight fetches
/// so we don't hammer the CDN). A failed fetch → `None` → the row
/// renders without an icon; icons are UX polish, not launch-critical.
pub async fn prefetch_icons(cache: &IconCache, rows: &[HitRow]) -> Vec<Option<PathBuf>> {
    let mut out: Vec<Option<PathBuf>> = vec![None; rows.len()];
    let results: Vec<(usize, Option<PathBuf>)> = futures::stream::iter(rows.iter().enumerate())
        .map(|(i, row)| async move {
            let Some(url) = row.icon_url.as_ref() else {
                return (i, None);
            };
            match cache.get(url).await {
                Ok(path) => (i, Some(path)),
                Err(err) => {
                    tracing::debug!("icon fetch failed for row {i} ({url}): {err}");
                    (i, None)
                }
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    for (i, path) in results {
        out[i] = path;
    }
    out
}

/// Render one hit inline: icon on the left, 2-line text block indented
/// past it. Falls through to plain text when `icon` is `None`.
fn print_hit_with_icon(row: &HitRow, icon: Option<&std::path::Path>) {
    use crossterm::{ExecutableCommand, cursor::MoveRight};

    let indent = match icon {
        Some(path) => {
            let cfg = viuer::Config {
                width: Some(ICON_CELLS_WIDE),
                height: Some(ICON_CELLS_TALL),
                absolute_offset: false,
                restore_cursor: true,
                transparent: true,
                ..Default::default()
            };
            match viuer::print_from_file(path, &cfg) {
                Ok((w, _)) => w as u16 + ICON_TEXT_GAP,
                Err(err) => {
                    tracing::debug!("viuer failed for {}: {err}", path.display());
                    0
                }
            }
        }
        None => 0,
    };

    let mut stdout = std::io::stdout();
    if indent > 0 {
        let _ = stdout.execute(MoveRight(indent));
    }
    let header = format!(
        "{title} · {slug} · {downloads}",
        title = row.title,
        slug = row.slug,
        downloads = format_downloads(row.downloads),
    );
    println!("{header}");

    // Always emit a second line even when the summary is empty, so the
    // next entry's cursor sits below the icon's 2-cell height rather than
    // overlapping it.
    if indent > 0 {
        let _ = stdout.execute(MoveRight(indent));
    }
    if row.summary.is_empty() {
        println!();
    } else {
        let summary = truncate(&row.summary, SUMMARY_MAX_CHARS);
        println!("  {}", dim(&summary));
    }
}

/// Interactive picker for `mr find` / `cf find`. Returns the chosen
/// `HitRow` — callers pull `.slug` off it to build the follow-up `Add`.
///
/// inquire's `Select` is blocking; we hop off the tokio runtime with
/// `spawn_blocking` to keep async callers safe.
pub async fn pick_hit(
    rows: Vec<HitRow>,
    prompt: &'static str,
    page_size: usize,
) -> anyhow::Result<HitRow> {
    let choices: Vec<HitChoice> = rows.into_iter().map(HitChoice::from_row).collect();
    let picked = tokio::task::spawn_blocking(move || {
        Select::new(prompt, choices)
            .with_page_size(page_size)
            .with_help_message("↑↓ navigate • type to filter • enter to select")
            .prompt()
    })
    .await??;
    Ok(picked.row)
}

/// inquire filters items by their `Display` output, so we pre-render the
/// two-line label once and keep it alongside the row.
struct HitChoice {
    row: HitRow,
    label: String,
}

impl HitChoice {
    fn from_row(row: HitRow) -> Self {
        let label = render_hit(&row);
        HitChoice { row, label }
    }
}

impl Display for HitChoice {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Compact download counts: `77.0M`, `31.8k`, `310`. Both API sites use
/// the same abbreviation on their own web UIs, so users read it fluently.
fn format_downloads(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Truncate on character boundaries — both Modrinth descriptions and
/// CurseForge summaries can contain multi-byte characters, and a
/// byte-based slice would panic mid-codepoint.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
