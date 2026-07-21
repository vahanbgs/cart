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

#[derive(Debug, Deserialize)]
pub struct Project {
    pub slug: String,
    pub title: String,
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
