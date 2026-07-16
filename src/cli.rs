use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    #[arg(long)]
    pub manifest: Option<PathBuf>,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,
}
