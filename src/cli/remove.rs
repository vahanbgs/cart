use crate::{config::Config, manifest};

use super::{Cli, Remove};

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
