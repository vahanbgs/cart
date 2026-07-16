use serde::Deserialize;

use crate::Cli;

#[derive(Debug, Deserialize)]
pub struct MinecraftVersion(String);

impl Default for MinecraftVersion {
    fn default() -> Self {
        Self("latest".to_owned())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CartManifest {
    #[serde(default)]
    pub minecraft: MinecraftVersion,
}

impl CartManifest {
    pub fn override_with(&mut self, cli: &Cli) {
        if let Some(minecraft_version) = &cli.minecraft_version {
            self.minecraft = MinecraftVersion(minecraft_version.to_owned());
        }
    }

    pub fn minecraft_version(&self) -> &str {
        &self.minecraft.0
    }
}
