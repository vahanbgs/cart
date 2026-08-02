use std::io::IsTerminal;
use std::path::PathBuf;

use cart::api::{curseforge, modrinth};
use futures::stream::StreamExt;
use url::Url;

use crate::cli::icon_cache::IconCache;

/// Backend-agnostic search hit. Both Modrinth and CurseForge search
/// responses collapse to this shape via the `From` impls below — every
/// field-name difference (`title`/`name`, `description`/`summary`,
/// `downloads`/`download_count`) is bridged in one place so the renderer
/// stays backend-blind.
///
/// `Clone` so the live-search picker can stash hits in its per-session
/// query cache and hand out fresh copies on cache hit without re-fetching.
#[derive(Clone)]
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

/// Wrap `s` in an ANSI "dim" (faint) SGR pair when stdout is a real
/// terminal, otherwise return it verbatim. Kept manual to avoid pulling
/// in a color crate for one attribute.
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
        prefetch_icons(cache, icon_urls(rows)).await
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

/// Prefetch a batch of icons concurrently (bounded to 8 in-flight
/// fetches so we don't hammer the CDN). A failed fetch → `None` → the
/// row renders without an icon; icons are UX polish, not launch-critical.
///
/// Takes owned `Vec<Option<Url>>` instead of `&[HitRow]` on purpose:
/// the picker's live-search path awaits this future inside a
/// `tokio::spawn`, and rustc's HRTB inference for closures over
/// borrowed slices fails to prove `Send` there. Owning the URLs sidesteps
/// the borrow entirely.
pub async fn prefetch_icons(cache: &IconCache, urls: Vec<Option<Url>>) -> Vec<Option<PathBuf>> {
    let mut out: Vec<Option<PathBuf>> = vec![None; urls.len()];
    let results: Vec<(usize, Option<PathBuf>)> =
        futures::stream::iter(urls.into_iter().enumerate())
            .map(|(i, url)| async move {
                let Some(url) = url else {
                    return (i, None);
                };
                match cache.get(&url).await {
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

/// Convenience: pull each row's icon URL for handing to `prefetch_icons`.
/// Kept here so both `hit_view` renderers and the picker share one
/// conversion.
pub fn icon_urls(rows: &[HitRow]) -> Vec<Option<Url>> {
    rows.iter().map(|r| r.icon_url.clone()).collect()
}

/// Render one hit inline: 2-line text block on the right, icon
/// overlaid in the left gutter. Falls through to plain text when
/// `icon` is `None`.
///
/// The two-phase order is deliberate: we print the text *first* to
/// force any needed vertical scroll (near the bottom of the screen),
/// then jump the cursor back up and draw the icon in the space we
/// just reserved. `viuer`'s own `restore_cursor: true` doesn't survive
/// scrolling — the saved position becomes stale — which is why we do
/// the save/restore manually around the icon draw, at a point where
/// scrolling can no longer happen.
fn print_hit_with_icon(row: &HitRow, icon: Option<&std::path::Path>) {
    use crossterm::{
        ExecutableCommand,
        cursor::{MoveRight, MoveToPreviousLine, RestorePosition, SavePosition},
    };

    let indent: u16 = if icon.is_some() {
        ICON_CELLS_WIDE as u16 + ICON_TEXT_GAP
    } else {
        0
    };

    let mut stdout = std::io::stdout();

    // Phase 1 — text. Header line, then a summary line (or blank so the
    // block is always 2 rows tall, matching the icon's height and giving
    // the "next entry" cursor a stable place to land).
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

    if indent > 0 {
        let _ = stdout.execute(MoveRight(indent));
    }
    if row.summary.is_empty() {
        println!();
    } else {
        let summary = truncate(&row.summary, SUMMARY_MAX_CHARS);
        println!("  {}", dim(&summary));
    }

    // Phase 2 — icon. Cursor is now on the row after the text block; any
    // scrolling has already happened. Save that row, walk up two lines,
    // draw the icon (which overlays the text's left padding), then restore
    // to the saved row so the next entry starts in the right place.
    if let Some(path) = icon {
        let _ = stdout.execute(SavePosition);
        let _ = stdout.execute(MoveToPreviousLine(ICON_CELLS_TALL as u16));
        let cfg = viuer::Config {
            width: Some(ICON_CELLS_WIDE),
            height: Some(ICON_CELLS_TALL),
            absolute_offset: false,
            restore_cursor: false,
            transparent: true,
            ..Default::default()
        };
        if let Err(err) = viuer::print_from_file(path, &cfg) {
            tracing::debug!("viuer failed for {}: {err}", path.display());
        }
        let _ = stdout.execute(RestorePosition);
    }
}

/// Compact download counts: `77.0M`, `31.8k`, `310`. Both API sites use
/// the same abbreviation on their own web UIs, so users read it fluently.
pub(super) fn format_downloads(n: u64) -> String {
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
pub(super) fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
