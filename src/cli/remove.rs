use clap::Args;

use crate::{config::Config, manifest};

use super::Cli;

#[derive(Args)]
pub struct Remove {
    /// Name of the mod as it appears under `[mods]` in `cart.toml`.
    pub name: String,
}

impl Remove {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let mut document = manifest::load_document(&path).await?;
        manifest::remove_mod(&mut document, &self.name)?;
        manifest::save_document(&path, &document).await?;

        Ok(())
    }
}
