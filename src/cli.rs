mod build;
mod init;
mod run;

pub use build::Build;
pub use init::Init;
pub use run::Run;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    Build(Build),
    Run(Run),
}
