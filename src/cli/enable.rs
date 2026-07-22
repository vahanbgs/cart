use crate::{config::Config, manifest};

use super::{Cli, Enable};

impl Enable {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let mut document = manifest::load_document(&path).await?;
        manifest::set_mod_disabled(&mut document, &self.name, false)?;
        manifest::save_document(&path, &document).await?;

        Ok(())
    }
}
