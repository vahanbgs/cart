use std::path::PathBuf;

use clap::Args;
use tokio::fs;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::manifest;

use super::Cli;

#[derive(Args)]
pub struct Init {
    pub path: PathBuf,
}

impl Init {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        fs::create_dir_all(&self.path).await?;

        let minecraft_version = cli.minecraft_version.as_deref().unwrap_or("latest");

        let mut document = DocumentMut::new();
        document["minecraft"] = value(minecraft_version);
        let mut mods = Table::new();
        mods.set_implicit(false);
        document["mods"] = Item::Table(mods);

        manifest::save_document(&self.path.join("cart.toml"), &document).await?;

        Ok(())
    }
}
