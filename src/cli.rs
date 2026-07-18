use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,
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
