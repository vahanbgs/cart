use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::AsRefStr;
use url::Url;

use crate::{Sha1Digest, api::Endpoint};

use super::{BASE_URL, DownloadEntry};

#[derive(Debug, Deserialize)]
pub struct JavaDistributionManifest(
    pub HashMap<JavaPlatform, HashMap<JavaVersionComponent, Vec<JavaDistributionInfo>>>,
);

impl Endpoint for JavaDistributionManifest {
    fn url() -> &'static Url {
        static URL: LazyLock<Url> = LazyLock::new(|| {
            BASE_URL
                .join("v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json")
                .unwrap()
        });

        &URL
    }
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
pub struct JavaDistributionInfo {
    pub availability: JavaDistributionAvailability,
    pub manifest: JavaDistributionManifestInfo,
    pub version: JavaDistributionVersion,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionAvailability {
    pub group: u16,
    pub progress: u8,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionManifestInfo {
    pub sha1: Sha1Digest,
    pub size: u32,
    pub url: Url,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistributionVersion {
    pub name: String,
    pub released: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct JavaDistribution {
    pub files: HashMap<PathBuf, FileSystemEntry>,
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
pub struct JavaDownloadOptions {
    pub lzma: Option<DownloadEntry>,
    pub raw: DownloadEntry,
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
