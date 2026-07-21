mod java;
mod rule;
mod version;
mod version_manifest;

pub use java::{
    FileSystemEntry, JavaDistribution, JavaDistributionManifest, JavaPlatform, JavaVersion,
    JavaVersionComponent,
};
pub use rule::{Action, Os, OsName, Rule};
pub use version::{
    Argument, ArgumentValue, Arguments, AssetIndex, GameJarDownloadOptions, Kind, Version,
};
pub use version_manifest::VersionManifest;

use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use serde::{Deserialize, Deserializer};
use strum::AsRefStr;
use url::Url;

use crate::Sha1Digest;

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://piston-meta.mojang.com/").unwrap());

#[derive(Debug, Deserialize)]
pub struct DownloadEntry {
    pub sha1: Sha1Digest,
    pub size: u32,
    pub url: Url,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Deserialize, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum NativeClassifier {
    NativesLinux,
    #[serde(alias = "natives-osx")]
    NativesMacos,
    // NativesOsx,
    NativesWindows,
    #[serde(rename = "natives-windows-32")]
    #[strum(serialize = "natives-windows-32")]
    NativesWindows32,
    #[serde(rename = "natives-windows-64")]
    #[strum(serialize = "natives-windows-64")]
    #[serde(alias = "natives-windows-${arch}")]
    NativesWindows64,
    // #[serde(rename = "natives-windows-${arch}")]
    // NativesWindowsArch,
}

impl NativeClassifier {
    pub const fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::NativesLinux
        }

        #[cfg(target_os = "macos")]
        {
            Self::NativesMacos
        }

        #[cfg(target_os = "windows")]
        {
            if cfg!(target_pointer_width = "64") {
                Self::NativesWindows64
            } else {
                Self::NativesWindows32
            }
        }
    }

    pub fn matches_current_platform(self) -> bool {
        match self {
            Self::NativesLinux => {
                cfg!(target_os = "linux")
            }
            Self::NativesMacos /* | Self::NativesOsx */ => {
                cfg!(target_os = "macos")
            }
            Self::NativesWindows => {
                cfg!(target_os = "windows")
            }
            Self::NativesWindows32 => {
                cfg!(target_os = "windows") && std::mem::size_of::<usize>() == 4
            }
            Self::NativesWindows64 => {
                cfg!(target_os = "windows") && std::mem::size_of::<usize>() == 8
            } // Self::NativesWindowsArch => {
              //     cfg!(all(target_os = "windows"))
              // }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AssetObject {
    pub hash: Sha1Digest,
    pub size: u32,
}

#[derive(Debug, Deserialize)]
pub struct AssetManifest {
    #[serde(default)]
    pub map_to_resources: bool,
    pub objects: HashMap<PathBuf, AssetObject>,
}

fn deserialize_url_or_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Url>, D::Error> {
    let s = String::deserialize(d)?;
    if s.is_empty() {
        return Ok(None);
    }
    Url::parse(&s).map(Some).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize)]
pub struct LibraryDownloadEntry {
    pub path: PathBuf,
    pub sha1: Sha1Digest,
    pub size: u32,
    #[serde(deserialize_with = "deserialize_url_or_none")]
    pub url: Option<Url>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LibraryDownloadOptions {
    pub artifact: Option<LibraryDownloadEntry>,
    pub classifiers: Option<HashMap<NativeClassifier, LibraryDownloadEntry>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Extract {
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LibraryEntry {
    #[serde(default)]
    pub downloads: LibraryDownloadOptions,
    pub extract: Option<Extract>,
    pub name: String,
    pub natives: Option<HashMap<OsName, NativeClassifier>>,
    pub rules: Option<Vec<Rule>>,
    /// Legacy library base URL (pre-1.13 Forge). Combined with the Maven
    /// coordinate of `name` to form the full download URL.
    #[serde(default)]
    pub url: Option<Url>,
    /// Legacy Forge field; `Some(false)` means server-only, skip on client.
    pub clientreq: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of platform-tagged classifiers is that we pick one
    /// automatically. This is the self-consistency test — if any future
    /// arch is added but the cfg branches in `current()` aren't updated,
    /// this catches it on that arch.
    #[test]
    fn native_classifier_current_matches_current_platform() {
        assert!(NativeClassifier::current().matches_current_platform());
    }

    /// Same guarantee for the shorter `OsName` — used in library `natives`
    /// maps to pick which classifier to extract.
    #[test]
    fn each_os_name_matches_exactly_one_platform() {
        let matches: Vec<_> = [OsName::Linux, OsName::Osx, OsName::Windows]
            .into_iter()
            .filter(|os| os.matches_current_platform())
            .collect();
        assert_eq!(matches.len(), 1, "expected exactly one OsName to match");
    }

    /// Modern manifests use `natives-osx` on some libraries and
    /// `natives-macos` on others; both must map to the same enum value.
    #[test]
    fn native_classifier_accepts_osx_alias() {
        let osx: NativeClassifier = serde_json::from_str("\"natives-osx\"").unwrap();
        let macos: NativeClassifier = serde_json::from_str("\"natives-macos\"").unwrap();
        assert_eq!(osx, NativeClassifier::NativesMacos);
        assert_eq!(macos, NativeClassifier::NativesMacos);
    }
}
