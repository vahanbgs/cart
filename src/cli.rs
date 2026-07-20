use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tokio::fs;

use crate::manifest::Manifest;

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,

    #[command(subcommand)]
    pub command: Subcommands,
}

impl Cli {
    pub fn manifest_path(&self) -> Option<(PathBuf, PathBuf)> {
        if let Some(path) = &self.manifest {
            path.parent()
                .map(|parent| (parent.to_owned(), path.to_owned()))
        } else {
            None
        }
    }
}

#[derive(Subcommand)]
pub enum Subcommands {
    Init(Init),
    Run,
}

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
