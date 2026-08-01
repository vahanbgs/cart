use std::sync::LazyLock;

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Deserializer};
use url::Url;

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://api.curseforge.com/").unwrap());

/// CurseForge's game id for Minecraft. Baked into every project/file query.
pub const MINECRAFT_GAME_ID: u32 = 432;

/// `modLoaderType` values used by CurseForge. Cart only exercises `Forge`
/// (mirroring the current Modrinth loader logic) but the rest are here
/// so a future Fabric/NeoForge lift is a one-line addition.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum LoaderType {
    Forge = 1,
    Fabric = 4,
    Quilt = 5,
    NeoForge = 6,
}

impl From<crate::LoaderKind> for LoaderType {
    fn from(kind: crate::LoaderKind) -> Self {
        match kind {
            crate::LoaderKind::Fabric => Self::Fabric,
            crate::LoaderKind::Forge => Self::Forge,
            crate::LoaderKind::NeoForge => Self::NeoForge,
        }
    }
}

/// `sortField` values used by `/v1/mods/search`. Cart pins `Popularity`
/// for the user-facing search — the rest are listed so callers don't
/// need to hunt the CurseForge docs for the numeric values if they ever
/// want a different sort.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum SortField {
    Featured = 1,
    Popularity = 2,
    LastUpdated = 3,
    Name = 4,
    Author = 5,
    TotalDownloads = 6,
    Category = 7,
    GameVersion = 8,
}

/// `GET /v1/mods/search?gameId=432&slug=<slug>` — narrow lookup used by
/// `find_project_by_slug` to turn a human slug into a stable numeric
/// project id. Not for user-visible search — that's `search_url`.
pub fn slug_search_url(slug: &str) -> Url {
    let mut url = BASE_URL.join("v1/mods/search").unwrap();
    url.query_pairs_mut()
        .append_pair("gameId", &MINECRAFT_GAME_ID.to_string())
        .append_pair("slug", slug);
    url
}

/// `GET /v1/mods/search?gameId=432&classId=6&searchFilter=<q>&sortField=2&sortOrder=desc&pageSize=<n>[&gameVersion=<mc>][&modLoaderType=<n>]`
/// — the mod-class subset of CurseForge's full-text search. Used by
/// `cart curseforge search` / `find`. `classId=6` is CurseForge's "Mods"
/// class, pinned so the query doesn't surface shader-pack /
/// resource-pack / modpack hits. `sortField=2` is Popularity — without
/// it CurseForge falls back to Featured ordering, which pushes
/// unrelated projects above exact-slug matches (typing `jei` doesn't
/// get JEI on top). This mirrors what CurseForge's own web UI does for
/// the same query. `gameVersion` + `modLoaderType` mirror `files_url`
/// and are filtered server-side by CurseForge, so we only surface mods
/// with a compatible file — verified against real API behaviour.
pub fn search_url(
    query: &str,
    limit: u32,
    minecraft_version: &str,
    loader: Option<LoaderType>,
) -> Url {
    let mut url = BASE_URL.join("v1/mods/search").unwrap();
    let mut query_pairs = url.query_pairs_mut();
    query_pairs
        .append_pair("gameId", &MINECRAFT_GAME_ID.to_string())
        .append_pair("classId", "6")
        .append_pair("searchFilter", query)
        .append_pair("sortField", &(SortField::Popularity as u32).to_string())
        .append_pair("sortOrder", "desc")
        .append_pair("pageSize", &limit.to_string())
        .append_pair("gameVersion", minecraft_version);
    if let Some(loader) = loader {
        query_pairs.append_pair("modLoaderType", &(loader as u32).to_string());
    }
    drop(query_pairs);
    url
}

/// `GET /v1/mods/{id}` — direct project lookup by numeric id. Used by
/// dep resolution to turn a `FileDependency.mod_id` into the slug that
/// keys the manifest entry.
pub fn project_url(project_id: u32) -> Url {
    BASE_URL
        .join(&format!("v1/mods/{project_id}"))
        .unwrap()
}

/// `GET /v1/mods/{id}/files?gameVersion=<mc>[&modLoaderType=<n>]` —
/// listed newest-first. Used by `cart update` and by loose `cart add`
/// to pick a starting file id.
pub fn files_url(project_id: u32, minecraft_version: &str, loader: Option<LoaderType>) -> Url {
    let mut url = BASE_URL
        .join(&format!("v1/mods/{project_id}/files"))
        .unwrap();
    let mut query = url.query_pairs_mut();
    query.append_pair("gameVersion", minecraft_version);
    if let Some(loader) = loader {
        query.append_pair("modLoaderType", &(loader as u32).to_string());
    }
    drop(query);
    url
}

/// `GET /v1/mods/{id}/files/{fileId}` — direct fetch for a pinned
/// `{ curseforge = <projectId>, file = <fileId> }` entry at build time.
pub fn file_url(project_id: u32, file_id: u32) -> Url {
    BASE_URL
        .join(&format!("v1/mods/{project_id}/files/{file_id}"))
        .unwrap()
}

/// CurseForge wraps every response in `{"data": ...}`. Kept internal —
/// callers deal in `Mod`/`File` directly.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

/// A CurseForge project — the "mod" in their terminology. Only the
/// fields cart consumes are kept; the API returns dozens more.
#[derive(Debug, Deserialize)]
pub struct Mod {
    pub id: u32,
    pub slug: String,
    pub name: String,
}

/// One search-page entry. Slimmed to what the CLI renders — CurseForge
/// returns thirty-plus fields per hit (screenshots, categories,
/// latestFiles, dateModified, links, …) that aren't shown, so ignoring
/// them at the serde layer keeps the parse cheap. `logo_url` is
/// flattened out of the nested `logo: { url, thumbnailUrl, ... }` object
/// (which itself may be `null` for projects without a logo).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: u32,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    pub download_count: u64,
    /// CurseForge always returns at least one entry, but treat it as
    /// optional against future schema drift.
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default, rename = "logo", deserialize_with = "logo_url")]
    pub logo_url: Option<Url>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Logo {
    #[serde(default)]
    url: Option<Url>,
    /// CurseForge pre-generates a 256×256 thumbnail alongside every
    /// upload. Prefer it over `url` when present — the full-res
    /// original is often 1024×1024+ and can take multiple seconds to
    /// download + decode for a picker preview.
    #[serde(default)]
    thumbnail_url: Option<Url>,
}

fn logo_url<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Url>, D::Error> {
    Ok(Option::<Logo>::deserialize(d)?.and_then(|l| l.thumbnail_url.or(l.url)))
}

impl SearchHit {
    /// First author's name, or `"—"` when the (unlikely) empty list case
    /// hits — the CLI needs a fixed placeholder to keep columns aligned.
    pub fn primary_author(&self) -> &str {
        self.authors.first().map(|a| a.name.as_str()).unwrap_or("—")
    }
}

#[derive(Debug, Deserialize)]
pub struct Author {
    pub name: String,
}

/// A specific released file for a project. `download_url` is `None`
/// when the project owner has disabled third-party API downloads — a
/// build-time failure we surface loudly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: u32,
    pub mod_id: u32,
    pub file_name: String,
    #[serde(default)]
    pub download_url: Option<Url>,
    pub file_date: DateTime<Utc>,
    pub hashes: Vec<FileHash>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<FileDependency>,
}

impl File {
    /// SHA-1 digest from the `hashes` array if present. CurseForge
    /// always ships both SHA-1 and MD5 in practice, but the API doesn't
    /// guarantee it — callers should treat `None` as "no integrity
    /// check available."
    pub fn sha1(&self) -> Option<&str> {
        self.hashes
            .iter()
            .find(|h| h.algo == HashAlgo::Sha1 as u8)
            .map(|h| h.value.as_str())
    }
}

#[derive(Debug, Deserialize)]
pub struct FileHash {
    pub value: String,
    pub algo: u8,
}

/// The numeric values CurseForge uses in `FileHash.algo`.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum HashAlgo {
    Sha1 = 1,
    Md5 = 2,
}

/// CurseForge's `relationType` values. `cart add` recursion only acts on
/// `RequiredDependency` and `OptionalDependency`; the rest are filtered
/// out at the CLI layer.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum RelationType {
    EmbeddedLibrary = 1,
    OptionalDependency = 2,
    RequiredDependency = 3,
    Tool = 4,
    Incompatible = 5,
    Include = 6,
}

/// One entry from a `File.dependencies` array. `relation_type` is kept
/// as `u8` on the wire (compared against `RelationType::… as u8`) so an
/// unknown CurseForge value doesn't fail the whole file deser — mirrors
/// how `HashAlgo::Sha1` is used against `FileHash.algo`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDependency {
    pub mod_id: u32,
    pub relation_type: u8,
}

/// Build a reqwest `Client` pre-loaded with the `x-api-key` header. All
/// CurseForge endpoints require it — a missing/invalid key returns
/// 403. Marked sensitive so proxies don't log it.
pub fn client(api_key: &str) -> anyhow::Result<Client> {
    let mut headers = header::HeaderMap::new();
    let mut key = header::HeaderValue::from_str(api_key)
        .context("CURSEFORGE_API_KEY contains characters invalid in an HTTP header")?;
    key.set_sensitive(true);
    headers.insert("x-api-key", key);
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );
    Client::builder()
        .default_headers(headers)
        .build()
        .context("build curseforge http client")
}

/// Fetch a project by its numeric id. `find_project_by_slug`'s companion
/// for the dep-resolution path, where a `FileDependency` only gives us
/// the id.
pub async fn get_project(http: &Client, project_id: u32) -> anyhow::Result<Mod> {
    let response = http.get(project_url(project_id)).send().await?;
    if response.status() == StatusCode::NOT_FOUND {
        bail!("CurseForge project {project_id} not found");
    }
    let envelope: Envelope<Mod> = response.error_for_status()?.json().await?;
    Ok(envelope.data)
}

/// Resolve a slug to its project. `search` returns partial matches, so
/// we filter to an exact slug hit — otherwise a query for `jei` would
/// happily match `jei-tweaker` if the exact one weren't first.
pub async fn find_project_by_slug(http: &Client, slug: &str) -> anyhow::Result<Mod> {
    let envelope: Envelope<Vec<Mod>> = http
        .get(slug_search_url(slug))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    envelope
        .data
        .into_iter()
        .find(|m| m.slug == slug)
        .with_context(|| format!("slug '{slug}' not found on CurseForge"))
}

/// Full-text mod search, filtered to results with at least one file
/// compatible with `minecraft_version` and (when supplied) `loader`.
/// Capped at `limit` hits. Ordered by CurseForge's `Popularity` sort
/// descending — see `search_url` for why.
pub async fn search(
    http: &Client,
    query: &str,
    limit: u32,
    minecraft_version: &str,
    loader: Option<LoaderType>,
) -> anyhow::Result<Vec<SearchHit>> {
    let envelope: Envelope<Vec<SearchHit>> = http
        .get(search_url(query, limit, minecraft_version, loader))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(envelope.data)
}

/// Newest file compatible with the given Minecraft + loader. Used by
/// `cart update` (find newer than pinned) and by `cart add` to pick an
/// initial `file` id.
pub async fn latest_file(
    http: &Client,
    project_id: u32,
    minecraft_version: &str,
    loader: Option<LoaderType>,
) -> anyhow::Result<File> {
    let envelope: Envelope<Vec<File>> = http
        .get(files_url(project_id, minecraft_version, loader))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    envelope
        .data
        .into_iter()
        .max_by_key(|f| f.file_date)
        .with_context(|| {
            format!(
                "no CurseForge files for project {project_id} compatible with minecraft {minecraft_version}"
            )
        })
}

/// Fetch a specific file by (project, file) id. This is the build-time
/// entry point — a pinned manifest entry doesn't need slug resolution
/// or version filtering, just a straight lookup.
pub async fn fetch_file(http: &Client, project_id: u32, file_id: u32) -> anyhow::Result<File> {
    let response = http.get(file_url(project_id, file_id)).send().await?;
    if response.status() == StatusCode::NOT_FOUND {
        bail!("CurseForge file {file_id} not found for project {project_id}");
    }
    let envelope: Envelope<File> = response.error_for_status()?.json().await?;
    Ok(envelope.data)
}
