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

    /// Whether Forge has ever released for `mc_version`. `cart init` uses
    /// this to keep Forge out of the loader menu for MC versions Forge
    /// hasn't caught up to (typically fresh snapshots, or brand-new
    /// releases in the first weeks after they drop).
    pub fn supports_mc(&self, mc_version: &str) -> bool {
        self.promos.contains_key(&format!("{mc_version}-latest"))
            || self
                .promos
                .contains_key(&format!("{mc_version}-recommended"))
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

/// Installer URL candidates in preference order, paired with the effective
/// Forge version identifier that the winning URL implies. Most MC versions
/// yield a single `(identifier, url)` pair. 1.7.10 gets a second candidate
/// with a doubled `-1.7.10` suffix — Forge switched to that layout at
/// build 10.13.4.1448 and never reverted, so the plain form 404s on the
/// promoted "recommended" build.
pub fn installer_url_candidates(forge_version: &str) -> Vec<(String, Url)> {
    let mut out = vec![(forge_version.to_owned(), installer_url(forge_version))];
    if forge_version.starts_with("1.7.10-") && !forge_version.ends_with("-1.7.10") {
        let doubled = format!("{forge_version}-1.7.10");
        let url = installer_url(&doubled);
        out.push((doubled, url));
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn promotions_with(entries: &[(&str, &str)]) -> ForgePromotions {
        ForgePromotions {
            promos: entries
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn supports_mc_matches_either_channel() {
        let p = promotions_with(&[
            ("1.20.1-recommended", "47.4.10"),
            ("1.20.1-latest", "47.4.22"),
            ("1.7.10-recommended", "10.13.4.1614"),
        ]);
        assert!(p.supports_mc("1.20.1"));
        assert!(p.supports_mc("1.7.10"));
        assert!(!p.supports_mc("25w40a"));
    }
}
