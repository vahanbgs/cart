use std::{collections::HashMap, sync::LazyLock};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

use crate::{
    Sha1Digest,
    api::{
        Endpoint,
        piston::{BASE_URL, version::Kind},
    },
};

#[derive(Debug, Deserialize)]
pub struct VersionManifest {
    latest: Latest,
    versions: Vec<VersionInfo>,
}

impl VersionManifest {
    pub fn latest_release(&self) -> &str {
        &self.latest.release
    }

    pub fn latest_snapshot(&self) -> &str {
        &self.latest.snapshot
    }

    pub fn versions(&self) -> &[VersionInfo] {
        &self.versions
    }

    pub fn version_map(&self) -> HashMap<&str, &VersionInfo> {
        self.versions()
            .iter()
            .map(|version| (&version.id[..], version))
            .collect()
    }
}

impl Endpoint for VersionManifest {
    fn url() -> &'static Url {
        static URL: LazyLock<Url> =
            LazyLock::new(|| BASE_URL.join("mc/game/version_manifest_v2.json").unwrap());

        &URL
    }
}

#[derive(Debug, Deserialize)]
struct Latest {
    release: String,
    snapshot: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub url: Url,
    pub time: DateTime<Utc>,
    #[serde(rename = "releaseTime")]
    pub release_time: DateTime<Utc>,
    pub sha1: Sha1Digest,
}
