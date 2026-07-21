use std::{collections::HashMap, sync::LazyLock};

use serde::Deserialize;
use url::Url;

use crate::api::Endpoint;

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
    /// Resolves a forge version channel ("latest" or "recommended") or a specific forge version
    /// number to a full combined version string like "1.20.1-47.3.12".
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

/// Returns the installer JAR URL for a full forge version string (e.g. "1.20.1-47.3.12").
pub fn installer_url(version: &str) -> Url {
    BASE_URL
        .join(&format!(
            "net/minecraftforge/forge/{version}/forge-{version}-installer.jar"
        ))
        .unwrap()
}
