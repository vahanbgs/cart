use std::sync::LazyLock;

use chrono::{DateTime, Utc};
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

#[derive(Debug, Deserialize)]
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
