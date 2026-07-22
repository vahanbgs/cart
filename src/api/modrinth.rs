use std::sync::LazyLock;

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use url::Url;

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://api.modrinth.com/").unwrap());

/// `GET /v2/project/{slug}` — validates that the slug exists.
pub fn project_url(slug: &str) -> Url {
    BASE_URL.join(&format!("v2/project/{slug}")).unwrap()
}

/// `GET /v2/project/{slug}/version?game_versions=[…]&loaders=[…]` — the
/// server-side filter narrows the response to versions we could actually
/// use, so we don't have to walk the full history.
pub fn versions_url(slug: &str, game_version: &str, loader: &str) -> Url {
    let mut url = BASE_URL
        .join(&format!("v2/project/{slug}/version"))
        .unwrap();
    url.query_pairs_mut()
        .append_pair("game_versions", &format!(r#"["{game_version}"]"#))
        .append_pair("loaders", &format!(r#"["{loader}"]"#));
    url
}

/// `GET /v2/search?query=<q>&facets=[["project_type:mod"]]&limit=<n>` —
/// the mod-typed subset of Modrinth's global project search. Used by
/// `cart modrinth search`. Facets are always narrowed to `project_type:mod`
/// so we don't surface shader/resource-pack/modpack hits from a bare
/// mod-name query.
pub fn search_url(query: &str, limit: u32) -> Url {
    let mut url = BASE_URL.join("v2/search").unwrap();
    url.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("facets", r#"[["project_type:mod"]]"#)
        .append_pair("limit", &limit.to_string());
    url
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub slug: String,
    pub title: String,
}

/// One page of `/v2/search` results. The API also returns `offset`,
/// `limit`, and `total_hits` — cart doesn't paginate today, so only
/// `hits` is decoded.
#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
}

/// One search-page entry. Slimmed to what the CLI renders — Modrinth
/// returns two dozen extra fields (categories, icon URL, gallery,
/// license, per-loader compat, colours) that aren't shown, so decoding
/// them costs allocations for no gain.
#[derive(Debug, Deserialize)]
pub struct SearchHit {
    pub slug: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub downloads: u64,
}

#[derive(Debug, Deserialize)]
pub struct Version {
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub date_published: DateTime<Utc>,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<VersionDependency>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionFile {
    pub url: Url,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Deserialize)]
pub struct VersionDependency {
    pub project_id: Option<String>,
    pub dependency_type: DependencyType,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// One resolved Modrinth version — a specific file at a specific version,
/// with everything callers need for both `cart add` (project title,
/// dependencies for warnings) and `cart build` (URL to fetch).
pub struct ResolvedVersion {
    pub project_slug: String,
    pub project_title: String,
    pub version_number: String,
    pub file: VersionFile,
    pub dependencies: Vec<VersionDependency>,
}

/// Resolve a `(slug, optional pinned version, mc, loader)` tuple to a
/// concrete file. Shared by `cart add` (pins by writing the returned
/// `version_number`) and `cart build` (fetches the returned `file.url`).
pub async fn resolve(
    http: &Client,
    slug: &str,
    version: Option<&str>,
    minecraft_version: &str,
    loader: &str,
) -> anyhow::Result<ResolvedVersion> {
    let project_response = http.get(project_url(slug)).send().await?;
    if project_response.status() == StatusCode::NOT_FOUND {
        bail!("slug '{slug}' not found on Modrinth");
    }
    let project: Project = project_response.error_for_status()?.json().await?;

    let versions: Vec<Version> = http
        .get(versions_url(slug, minecraft_version, loader))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let picked = match version {
        Some(pin) => versions
            .into_iter()
            .find(|v| v.version_number == pin)
            .with_context(|| {
                format!(
                    "no version '{pin}' of '{slug}' compatible with minecraft {minecraft_version} + {loader}"
                )
            })?,
        None => versions.into_iter().max_by_key(|v| v.date_published).with_context(|| {
            format!(
                "no version of '{slug}' compatible with minecraft {minecraft_version} + {loader}"
            )
        })?,
    };

    let file = picked
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| picked.files.first())
        .with_context(|| format!("modrinth version {} has no files", picked.version_number))?
        .clone();

    Ok(ResolvedVersion {
        project_slug: project.slug,
        project_title: project.title,
        version_number: picked.version_number,
        file,
        dependencies: picked.dependencies,
    })
}

/// Full-text mod search, capped at `limit` hits. Ordering is Modrinth's
/// default (server-computed relevance).
pub async fn search(http: &Client, query: &str, limit: u32) -> anyhow::Result<Vec<SearchHit>> {
    let response: SearchResponse = http
        .get(search_url(query, limit))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response.hits)
}
