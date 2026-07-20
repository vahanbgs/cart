use std::path::PathBuf;

use clap::Args;
use tokio::fs;

use crate::manifest::Manifest;

use super::Cli;

#[derive(Args)]
pub struct Init {
    pub path: PathBuf,
}

impl Init {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        fs::create_dir_all(&self.path).await?;
        fs::write(
            self.path.join("cart.toml"),
            toml::to_string_pretty(&Manifest::new(&cli))?,
        )
        .await?;

        Ok(())
    }
}
