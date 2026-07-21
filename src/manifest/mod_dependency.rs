use serde::Deserialize;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ModDependency {
    /// Sourced from Modrinth by project slug. If `version` is `None`, the
    /// entry is "loose" — `cart build` resolves it to the newest version
    /// compatible with the manifest's Minecraft version + loader.
    Modrinth {
        modrinth: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        disabled: bool,
    },
    /// Raw URL — the escape hatch for anything not on Modrinth. Fully
    /// pinned by the URL; `cart update` skips these entries.
    Url {
        url: Url,
        #[serde(default)]
        disabled: bool,
    },
}

impl ModDependency {
    pub fn is_disabled(&self) -> bool {
        match self {
            Self::Modrinth { disabled, .. } | Self::Url { disabled, .. } => *disabled,
        }
    }

    /// Filename this mod should occupy in `minecraft/mods/`.
    pub fn filename(&self, name: &str) -> String {
        if self.is_disabled() {
            format!("{name}.jar.disabled")
        } else {
            format!("{name}.jar")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn parse(entry: &str) -> ModDependency {
        let snippet = format!("key = {entry}");
        let map: HashMap<String, ModDependency> = toml::from_str(&snippet).unwrap();
        map.into_iter().next().unwrap().1
    }

    #[test]
    fn modrinth_loose() {
        let ModDependency::Modrinth {
            modrinth,
            version,
            disabled,
        } = parse(r#"{ modrinth = "jei" }"#)
        else {
            panic!("expected Modrinth variant");
        };
        assert_eq!(modrinth, "jei");
        assert!(version.is_none());
        assert!(!disabled);
    }

    #[test]
    fn modrinth_pinned_and_disabled() {
        let ModDependency::Modrinth {
            modrinth,
            version,
            disabled,
        } = parse(r#"{ modrinth = "jei", version = "15.2.0.27", disabled = true }"#)
        else {
            panic!("expected Modrinth variant");
        };
        assert_eq!(modrinth, "jei");
        assert_eq!(version.as_deref(), Some("15.2.0.27"));
        assert!(disabled);
    }

    #[test]
    fn url_variant() {
        let ModDependency::Url { url, disabled } =
            parse(r#"{ url = "https://example.com/foo.jar" }"#)
        else {
            panic!("expected Url variant");
        };
        assert_eq!(url.as_str(), "https://example.com/foo.jar");
        assert!(!disabled);
    }

    #[test]
    fn filename_enabled_vs_disabled() {
        let enabled = parse(r#"{ modrinth = "jei" }"#);
        let disabled = parse(r#"{ modrinth = "jei", disabled = true }"#);
        assert_eq!(enabled.filename("jei"), "jei.jar");
        assert_eq!(disabled.filename("jei"), "jei.jar.disabled");
    }

    /// Untagged enums pick the first matching variant, so if both keys are
    /// present the Modrinth arm wins. Documenting this so a future variant
    /// reorder is a deliberate choice, not a silent regression.
    #[test]
    fn ambiguous_entry_resolves_to_modrinth() {
        let dep = parse(r#"{ modrinth = "jei", url = "https://example.com/x.jar" }"#);
        assert!(matches!(dep, ModDependency::Modrinth { .. }));
    }
}
