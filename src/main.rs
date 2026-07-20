mod cli;
mod config;
mod manifest;

use clap::Parser;

use cli::{Cli, Subcommands};
use manifest::Manifest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match &cli.command {
        Subcommands::Build(build) => {
            build.run(&cli).await?;
        }
        Subcommands::Init(init) => {
            init.run(&cli).await?;
        }
        Subcommands::Run(run) => {
            run.run(&cli).await?;
        }
    }

    Ok(())
}
