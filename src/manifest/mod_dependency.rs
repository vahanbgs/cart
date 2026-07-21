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
