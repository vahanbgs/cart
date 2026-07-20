mod java;
mod rule;
mod version;
mod version_manifest;

pub use java::{
    FileSystemEntry, JavaDistribution, JavaDistributionManifest, JavaPlatform, JavaVersion,
    JavaVersionComponent,
};
pub use rule::{Action, Os, OsName, Rule};
pub use version::{AssetIndex, GameJarDownloadOptions, Kind, Version};
pub use version_manifest::VersionManifest;

use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use serde::Deserialize;
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

#[derive(Debug, Deserialize)]
pub struct LibraryDownloadEntry {
    pub path: PathBuf,
    pub sha1: Sha1Digest,
    pub size: u32,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
pub struct LibraryDownloadOptions {
    pub artifact: Option<LibraryDownloadEntry>,
    pub classifiers: Option<HashMap<NativeClassifier, LibraryDownloadEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct Extract {
    pub exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LibraryEntry {
    pub downloads: LibraryDownloadOptions,
    pub extract: Option<Extract>,
    pub name: String,
    pub natives: Option<HashMap<OsName, NativeClassifier>>,
    pub rules: Option<Vec<Rule>>,
}
