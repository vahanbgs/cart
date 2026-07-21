mod build;
mod init;
mod list;
mod run;

pub use build::Build;
pub use init::Init;
pub use list::List;
pub use run::Run;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Cli {
    /// Run as if invoked from this directory. The cart project's `cart.toml`
    /// is expected to live directly inside it. Mirrors `make -C` / `cargo -C`.
    #[arg(short = 'C', long = "directory", value_name = "DIR", global = true)]
    pub directory: Option<PathBuf>,

    /// Increase log verbosity: default = info, `-v` = debug, `-vv` = trace.
    /// `RUST_LOG` overrides both if set.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,

    #[command(subcommand)]
    pub command: Subcommands,
}

impl Cli {
    /// Explicit `(project_dir, manifest_path)` pair when `-C` was passed.
    /// `None` means walk up from cwd looking for `cart.toml`.
    pub fn manifest_path(&self) -> Option<(PathBuf, PathBuf)> {
        self.directory
            .as_ref()
            .map(|dir| (dir.clone(), dir.join("cart.toml")))
    }
}

#[derive(Subcommand)]
pub enum Subcommands {
    Init(Init),
    Build(Build),
    Run(Run),
    List(List),
}
