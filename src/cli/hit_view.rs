use std::fmt::{self, Display, Formatter};
use std::io::IsTerminal;

use cart::api::{curseforge, modrinth};
use inquire::Select;

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
}

impl From<&modrinth::SearchHit> for HitRow {
    fn from(h: &modrinth::SearchHit) -> Self {
        HitRow {
            slug: h.slug.clone(),
            title: h.title.clone(),
            summary: h.description.clone(),
            downloads: h.downloads,
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

/// Non-interactive stdout output for `mr search` / `cf search`. A blank
/// line between hits gives the eye something to rest on when skimming a
/// long list.
pub fn print_search_results(rows: &[HitRow]) {
    let mut first = true;
    for row in rows {
        if !first {
            println!();
        }
        println!("{}", render_hit(row));
        first = false;
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
