//! Fabric metadata API (`meta.fabricmc.net/v2`).
//!
//! Two endpoints matter to us:
//!
//! - `GET /v2/versions/loader` — every published loader version, newest
//!   first. Used to resolve `LoaderSpec::Latest` → a concrete loader.
//! - `GET /v2/versions/loader/{mc}/{loader}/profile/json` — the launcher
//!   profile for a `(mc, loader)` combo. Same shape as Mojang's
//!   per-version manifest (`mainClass`, `arguments`, `libraries`,
//!   `inheritsFrom`) so the launch pipeline can consume it directly.

use std::sync::LazyLock;

use serde::Deserialize;
use url::Url;

use crate::Sha1Digest;
use crate::api::{Endpoint, piston::Arguments};

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://meta.fabricmc.net/").unwrap());

static LOADER_VERSIONS_URL: LazyLock<Url> =
    LazyLock::new(|| BASE_URL.join("v2/versions/loader").unwrap());

static GAME_VERSIONS_URL: LazyLock<Url> =
    LazyLock::new(|| BASE_URL.join("v2/versions/game").unwrap());

/// One entry from `/v2/versions/loader`. `stable=true` marks the loader
/// as the "stable" release channel — `LoaderSpec::Latest` picks the newest
/// stable entry.
#[derive(Clone, Debug, Deserialize)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
    pub maven: String,
    pub build: u32,
}

/// The `/v2/versions/loader` response — a flat list.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct LoaderVersions(pub Vec<LoaderVersion>);

impl Endpoint for LoaderVersions {
    fn url() -> &'static Url {
        &LOADER_VERSIONS_URL
    }
}

/// One entry from `/v2/versions/game` — every Minecraft version Fabric has
/// intermediary mappings for. Used to filter loader options in `cart init`
/// so we don't offer Fabric for MC versions it doesn't cover.
#[derive(Clone, Debug, Deserialize)]
pub struct GameVersion {
    pub version: String,
    pub stable: bool,
}

/// The `/v2/versions/game` response — a flat list.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct GameVersions(pub Vec<GameVersion>);

impl GameVersions {
    pub fn contains(&self, mc_version: &str) -> bool {
        self.0.iter().any(|v| v.version == mc_version)
    }
}

impl Endpoint for GameVersions {
    fn url() -> &'static Url {
        &GAME_VERSIONS_URL
    }
}

/// URL for `/v2/versions/loader/{mc_version}/{loader_version}/profile/json`.
pub fn profile_url(mc_version: &str, loader_version: &str) -> Url {
    BASE_URL
        .join(&format!(
            "v2/versions/loader/{mc_version}/{loader_version}/profile/json"
        ))
        .unwrap()
}

/// The `profile/json` response. Shape mirrors Mojang's per-version manifest
/// (`inheritsFrom` points at the base MC version so the launcher merges
/// libraries + args on top of vanilla) but the library entries use a flat
/// `{name, url, sha1, size}` shape rather than the nested
/// `downloads.artifact.*` that piston uses.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub inherits_from: String,
    pub main_class: String,
    pub arguments: Arguments,
    pub libraries: Vec<ProfileLibrary>,
}

/// One Fabric-style library entry. The full download URL is `url + Maven
/// coordinate path` — Fabric only writes the base host.
///
/// `sha1` is absent on the `fabric-loader` and `intermediary` self-references
/// in the profile — Fabric's own jars are pulled at runtime by KnotClient
/// via other means, and the profile omits their integrity metadata.
#[derive(Clone, Debug, Deserialize)]
pub struct ProfileLibrary {
    /// Maven coordinate, e.g. `org.ow2.asm:asm:9.10.1`.
    pub name: String,
    /// Base URL (host with trailing slash), e.g.
    /// `https://maven.fabricmc.net/`. Not a full download URL.
    pub url: Url,
    #[serde(default)]
    pub sha1: Option<Sha1Digest>,
    #[serde(default)]
    pub size: Option<u32>,
}
