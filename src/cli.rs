use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Init { path: PathBuf },
    Run,
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
