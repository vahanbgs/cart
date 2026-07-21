use clap::Args;

use crate::{config::Config, manifest};

use super::Cli;

#[derive(Args)]
pub struct Disable {
    /// Name of the mod as it appears under `[mods]` in `cart.toml`.
    pub name: String,
}

impl Disable {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let mut document = manifest::load_document(&path).await?;
        manifest::set_mod_disabled(&mut document, &self.name, true)?;
        manifest::save_document(&path, &document).await?;

        Ok(())
    }
}
