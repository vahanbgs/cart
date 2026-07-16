use std::{collections::HashMap, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::AsRefStr;
use url::Url;

use crate::Sha1Digest;

#[derive(Debug, Deserialize)]
pub struct LatestVersion {
    pub release: String,
    pub snapshot: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionType {
    OldAlpha,
    OldBeta,
    Release,
    Snapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JavaPlatform {
    Gamecore,
    Linux,
    LinuxI386,
    MacOs,
    MacOsArm64,
    WindowsArm64,
    WindowsX64,
    WindowsX86,
}

impl JavaPlatform {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub const CURRENT: Self = Self::Linux;

    #[cfg(all(target_os = "linux", target_arch = "x86"))]
    pub const CURRENT: Self = Self::LinuxI386;

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    pub const CURRENT: Self = Self::MacOs;

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub const CURRENT: Self = Self::MacOsArm64;

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    pub const CURRENT: Self = Self::WindowsArm64;

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub const CURRENT: Self = Self::WindowsX64;

    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    pub const CURRENT: Self = Self::WindowsX86;
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionAvailability {
    pub group: u16,
    pub progress: u8,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionInfo {
    pub availability: JavaDistributionAvailability,
    pub manifest: JavaDistributionManifestInfo,
    pub version: JavaDistributionVersion,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionVersion {
    pub name: String,
    pub released: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, AsRefStr)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum JavaVersionComponent {
    JavaRuntimeAlpha,
    JavaRuntimeBeta,
    JavaRuntimeDelta,
    JavaRuntimeEpsilon,
    JavaRuntimeGamma,
    JavaRuntimeGammaSnapshot,
    #[default]
    JreLegacy,
    MinecraftJavaExe,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsName {
    Linux,
    Osx,
    Windows,
}

impl OsName {
    pub fn matches_current_platform(self) -> bool {
        match self {
            OsName::Linux => cfg!(target_os = "linux"),
            OsName::Osx => cfg!(target_os = "macos"),
            OsName::Windows => cfg!(target_os = "windows"),
        }
    }
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
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub component: JavaVersionComponent,
    pub major_version: u8,
}

impl Default for JavaVersion {
    fn default() -> Self {
        Self {
            component: Default::default(),
            major_version: 8,
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
#[serde(rename_all = "camelCase")]
pub struct AssetIndex {
    pub id: String,
    pub sha1: Sha1Digest,
    pub size: u32,
    pub total_size: u32,
    pub url: Url,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Os {
    Arch { arch: Arch },
    Name { name: OsName },
}

#[derive(Debug, Deserialize)]
pub struct Features {
    pub is_demo_user: Option<bool>,
    pub is_quick_play_realms: Option<bool>,
    pub is_quick_play_singleplayer: Option<bool>,
    pub is_quick_play_multiplayer: Option<bool>,
    pub has_custom_resolution: Option<bool>,
    pub has_quick_plays_support: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub action: Action,
    pub os: Option<Os>,
    pub features: Option<Features>,
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    Multiple(Vec<String>),
    Simple(String),
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
pub enum Arguments {
    Modern { game: Vec<Argument> },
    Legacy(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionManifest {
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
    pub version_type: VersionType,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub version_type: VersionType,
    pub url: Url,
    pub time: DateTime<Utc>,
    #[serde(rename = "releaseTime")]
    pub release_time: DateTime<Utc>,
    pub sha1: Sha1Digest,
}

#[derive(Debug, Deserialize)]
pub struct VersionListManifest {
    pub latest: LatestVersion,
    pub versions: Vec<VersionInfo>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadEntry {
    pub sha1: Sha1Digest,
    pub size: u32,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
pub struct GameJarDownloadOptions {
    pub client: DownloadEntry,
    pub server: Option<DownloadEntry>,
    pub windows_server: Option<DownloadEntry>,
}

#[derive(Debug, Deserialize)]
pub struct JavaDownloadOptions {
    pub lzma: Option<DownloadEntry>,
    pub raw: DownloadEntry,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FileSystemEntry {
    Directory,
    File {
        downloads: JavaDownloadOptions,
        executable: bool,
    },
    Link {
        target: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionManifestInfo {
    pub sha1: Sha1Digest,
    pub size: u32,
    pub url: Url,
}

pub type JavaDistributionListManifest =
    HashMap<JavaPlatform, HashMap<JavaVersionComponent, Vec<JavaDistributionInfo>>>;

#[derive(Debug, Deserialize)]
pub struct JavaDistributionManifest {
    pub files: HashMap<PathBuf, FileSystemEntry>,
}
