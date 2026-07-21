use std::{collections::HashMap, path::PathBuf, sync::LazyLock};

use anyhow::bail;
use serde::Deserialize;
use url::Url;

use crate::api::{
    Endpoint,
    piston::{Argument, LibraryEntry},
};

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://maven.minecraftforge.net/").unwrap());

static PROMOTIONS_URL: LazyLock<Url> = LazyLock::new(|| {
    Url::parse("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
        .unwrap()
});

#[derive(Debug, Deserialize)]
pub struct ForgePromotions {
    promos: HashMap<String, String>,
}

impl ForgePromotions {
    pub fn resolve(&self, mc_version: &str, channel: &str) -> Option<String> {
        let forge_version = match channel {
            "latest" | "recommended" => self
                .promos
                .get(&format!("{mc_version}-{channel}"))
                .map(String::as_str)?,
            specific => specific,
        };

        Some(format!("{mc_version}-{forge_version}"))
    }
}

impl Endpoint for ForgePromotions {
    fn url() -> &'static Url {
        &PROMOTIONS_URL
    }
}

pub fn installer_url(version: &str) -> Url {
    BASE_URL
        .join(&format!(
            "net/minecraftforge/forge/{version}/forge-{version}-installer.jar"
        ))
        .unwrap()
}

pub struct MavenCoordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoordinate {
    /// Parses `group:artifact:version[:classifier][@extension]`, with or without
    /// surrounding `[` `]` brackets (used in install_profile data entries).
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let s = s
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(s);

        let (s, extension) = match s.rsplit_once('@') {
            Some((s, ext)) => (s, ext.to_owned()),
            None => (s, "jar".to_owned()),
        };

        let parts: Vec<&str> = s.splitn(4, ':').collect();

        match parts.as_slice() {
            [group, artifact, version] => Ok(Self {
                group: group.to_string(),
                artifact: artifact.to_string(),
                version: version.to_string(),
                classifier: None,
                extension,
            }),
            [group, artifact, version, classifier] => Ok(Self {
                group: group.to_string(),
                artifact: artifact.to_string(),
                version: version.to_string(),
                classifier: Some(classifier.to_string()),
                extension,
            }),
            _ => bail!("invalid Maven coordinate: {s}"),
        }
    }

    /// Returns the relative path for this artifact, e.g.
    /// `net/minecraftforge/forge/1.20.1-47.3.12/forge-1.20.1-47.3.12-universal.jar`.
    pub fn to_path(&self) -> PathBuf {
        let group_path = self.group.replace('.', "/");
        let filename = match &self.classifier {
            Some(cls) => format!(
                "{}-{}-{}.{}",
                self.artifact, self.version, cls, self.extension
            ),
            None => format!("{}-{}.{}", self.artifact, self.version, self.extension),
        };
        PathBuf::from(format!(
            "{}/{}/{}/{}",
            group_path, self.artifact, self.version, filename
        ))
    }
}

/// Present only in legacy (pre-1.13) install profiles.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyInstall {
    /// Maven coordinate of the Forge universal JAR (e.g. `net.minecraftforge:forge:1.12.2-14.23.5.2864:universal`).
    pub path: String,
    /// Filename of the universal JAR bundled inside the installer ZIP.
    pub file_path: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallProfile {
    #[serde(default)]
    pub minecraft: String,
    #[serde(default)]
    pub data: HashMap<String, DataEntry>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub libraries: Vec<LibraryEntry>,
    // Legacy format fields
    pub install: Option<LegacyInstall>,
    #[serde(rename = "versionInfo")]
    pub version_info: Option<ForgeVersion>,
}

/// A data entry holds client and server variants of a path or Maven coordinate.
#[derive(Debug, Deserialize)]
pub struct DataEntry {
    pub client: String,
    pub server: String,
}

#[derive(Debug, Deserialize)]
pub struct Processor {
    pub jar: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    pub args: Vec<String>,
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    pub sides: Option<Vec<String>>,
}

// ── version.json (inside installer) ──────────────────────────────────────────

/// The version manifest bundled inside the Forge installer.  It inherits from a
/// vanilla version and carries only the delta: extra libraries, main class, and
/// additional arguments.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeVersion {
    pub id: String,
    pub inherits_from: String,
    pub main_class: String,
    pub libraries: Vec<LibraryEntry>,
    #[serde(default)]
    pub arguments: ForgeArguments,
    /// Present on pre-1.13 Forge versions; takes precedence over `arguments`.
    pub minecraft_arguments: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ForgeArguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}
