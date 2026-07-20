use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

use crate::Sha1Digest;

use super::{DownloadEntry, JavaVersion, LibraryEntry, Rule};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    #[serde(alias = "minecraftArguments")]
    pub arguments: Arguments,
    pub asset_index: AssetIndex,
    pub assets: String,
    pub downloads: GameJarDownloadOptions,
    pub id: String,
    #[serde(default)]
    pub java_version: JavaVersion,
    pub libraries: Vec<LibraryEntry>,
    pub main_class: String,
    pub minimum_launcher_version: u8,
    #[serde(rename = "releaseTime")]
    pub release_time: DateTime<Utc>,
    pub time: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: Kind,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Arguments {
    Modern { game: Vec<Argument> },
    Legacy(String),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Complex {
        rules: Vec<Rule>,
        value: ArgumentValue,
    },
    Simple(String),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    Multiple(Vec<String>),
    Simple(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndex {
    pub id: String,
    pub sha1: Sha1Digest,
    pub size: u32,
    pub total_size: u32,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
pub struct GameJarDownloadOptions {
    pub client: DownloadEntry,
    pub server: Option<DownloadEntry>,
    pub windows_server: Option<DownloadEntry>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    OldAlpha,
    OldBeta,
    Release,
    Snapshot,
}
