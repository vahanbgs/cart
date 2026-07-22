//! NeoForge maven-metadata client.
//!
//! NeoForge doesn't publish a JSON promotions/versions endpoint like Forge
//! or Fabric — the source of truth is the maven-metadata.xml at
//! `maven.neoforged.net/releases/net/neoforged/neoforge/`. Cart parses
//! `<version>...</version>` entries with plain string extraction rather
//! than a full XML dep: the format is a flat list and the only nesting is
//! ambient elements we don't care about.
//!
//! Version scheme: for Minecraft `1.X.Y`, NeoForge versions look like
//! `X.Y.<build>` (e.g. `20.4.251` for MC 1.20.4). MC versions without a
//! patch (`1.21`) use `.0.` in the NeoForge prefix.

use std::sync::LazyLock;

use reqwest::Client;
use url::Url;

static BASE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://maven.neoforged.net/releases/").unwrap());

static MAVEN_METADATA_URL: LazyLock<Url> = LazyLock::new(|| {
    BASE_URL
        .join("net/neoforged/neoforge/maven-metadata.xml")
        .unwrap()
});

pub fn maven_metadata_url() -> &'static Url {
    &MAVEN_METADATA_URL
}

pub fn installer_url(version: &str) -> Url {
    BASE_URL
        .join(&format!(
            "net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar"
        ))
        .unwrap()
}

/// Parsed maven-metadata.xml — flat list of every published NeoForge
/// version.
pub struct MavenMetadata {
    versions: Vec<String>,
}

impl MavenMetadata {
    pub async fn fetch(http: &Client) -> anyhow::Result<Self> {
        let body = http
            .get(maven_metadata_url().clone())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(Self::parse(&body))
    }

    /// Pull `<version>...</version>` bodies out of the maven-metadata XML.
    /// Ambient elements (`<groupId>`, `<latest>`, etc.) are skipped because
    /// they don't use the same tag.
    pub fn parse(xml: &str) -> Self {
        const OPEN: &str = "<version>";
        const CLOSE: &str = "</version>";
        let mut versions = Vec::new();
        let mut rest = xml;
        while let Some(start) = rest.find(OPEN) {
            let after = &rest[start + OPEN.len()..];
            if let Some(end) = after.find(CLOSE) {
                versions.push(after[..end].to_owned());
                rest = &after[end + CLOSE.len()..];
            } else {
                break;
            }
        }
        Self { versions }
    }

    pub fn versions(&self) -> &[String] {
        &self.versions
    }

    /// Whether NeoForge has any release matching `mc_version`. Used by
    /// `cart init` to filter the loader menu.
    pub fn supports_mc(&self, mc_version: &str) -> bool {
        let Some(prefix) = neoforge_prefix(mc_version) else {
            return false;
        };
        self.versions.iter().any(|v| v.starts_with(&prefix))
    }

    /// Latest stable NeoForge version for `mc_version` — the entry under
    /// the MC's prefix with the highest build number, excluding `-beta`,
    /// `-alpha`, `-rc`, and `-pre` prerelease suffixes.
    ///
    /// The build number is parsed as `u32` rather than compared
    /// lexicographically because `20.4.100` sorts *lower* than `20.4.99`
    /// as strings but higher as versions.
    pub fn latest_stable_for_mc(&self, mc_version: &str) -> Option<String> {
        let prefix = neoforge_prefix(mc_version)?;
        self.versions
            .iter()
            .filter(|v| v.starts_with(&prefix))
            .filter(|v| !is_prerelease(v))
            .max_by_key(|v| {
                v.strip_prefix(&prefix)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0)
            })
            .cloned()
    }
}

/// `1.X` or `1.X.Y` → `X.Y.` (missing patch → `X.0.`). Returns None for
/// MC versions NeoForge can't cover — anything not matching `1.<num>[.<num>]`.
fn neoforge_prefix(mc_version: &str) -> Option<String> {
    let rest = mc_version.strip_prefix("1.")?;
    let (major, minor) = match rest.split_once('.') {
        Some((maj, min)) => (maj, min),
        None => (rest, "0"),
    };
    if !major.chars().all(|c| c.is_ascii_digit()) || !minor.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{major}.{minor}."))
}

fn is_prerelease(v: &str) -> bool {
    v.contains("-beta") || v.contains("-alpha") || v.contains("-rc") || v.contains("-pre")
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>net.neoforged</groupId>
  <artifactId>neoforge</artifactId>
  <versioning>
    <latest>21.1.242</latest>
    <release>21.1.242</release>
    <versions>
      <version>20.4.99</version>
      <version>20.4.100</version>
      <version>20.4.237</version>
      <version>20.4.238-beta</version>
      <version>21.1.240</version>
      <version>21.1.241</version>
      <version>21.1.242</version>
      <version>21.1.243-rc</version>
    </versions>
  </versioning>
</metadata>"#;

    #[test]
    fn parse_extracts_only_version_entries() {
        let m = MavenMetadata::parse(XML);
        assert_eq!(
            m.versions(),
            &[
                "20.4.99",
                "20.4.100",
                "20.4.237",
                "20.4.238-beta",
                "21.1.240",
                "21.1.241",
                "21.1.242",
                "21.1.243-rc",
            ],
            "the ambient <latest> and <release> elements must not leak in"
        );
    }

    #[test]
    fn supports_mc_matches_by_prefix() {
        let m = MavenMetadata::parse(XML);
        assert!(m.supports_mc("1.20.4"));
        assert!(m.supports_mc("1.21.1"));
        assert!(!m.supports_mc("1.18.2"));
        assert!(!m.supports_mc("b1.7"));
    }

    /// `20.4.100` > `20.4.99` as a version but < as a string. The
    /// resolver must compare the build number numerically or Modrinth
    /// searches paired with a NeoForge pin would silently down-rev.
    #[test]
    fn latest_stable_picks_highest_build_numerically() {
        let m = MavenMetadata::parse(XML);
        assert_eq!(
            m.latest_stable_for_mc("1.20.4"),
            Some("20.4.237".to_owned()),
            "excludes -beta and picks 237 (highest u32), not 100 (lex-max)"
        );
    }

    #[test]
    fn latest_stable_excludes_all_prerelease_suffixes() {
        let m = MavenMetadata::parse(XML);
        assert_eq!(
            m.latest_stable_for_mc("1.21.1"),
            Some("21.1.242".to_owned()),
            "-rc suffix must be filtered out alongside -beta"
        );
    }

    #[test]
    fn mc_without_patch_maps_to_zero_prefix() {
        assert_eq!(neoforge_prefix("1.21"), Some("21.0.".to_owned()));
        assert_eq!(neoforge_prefix("1.20.4"), Some("20.4.".to_owned()));
    }

    #[test]
    fn mc_without_leading_one_returns_none() {
        assert!(neoforge_prefix("b1.7.3").is_none());
        assert!(neoforge_prefix("2.0.0").is_none());
    }
}
