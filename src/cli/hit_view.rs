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
