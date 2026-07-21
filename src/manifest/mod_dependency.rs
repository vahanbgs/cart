use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ModDependency {
    Url {
        url: Url,
        /// Place the mod as `<name>.jar.disabled` so Forge/Fabric skips it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        disabled: bool,
    },
}

impl ModDependency {
    pub fn is_disabled(&self) -> bool {
        match self {
            Self::Url { disabled, .. } => *disabled,
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
