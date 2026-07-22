mod cli;
mod config;
mod manifest;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Subcommands};
use manifest::Manifest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let default_filter = match cli.verbose {
        0 => "cart=info",
        1 => "cart=debug",
        _ => "cart=trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .compact()
        .init();

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
        Subcommands::List(list) => {
            list.run(&cli).await?;
        }
        Subcommands::Remove(remove) => {
            remove.run(&cli).await?;
        }
        Subcommands::Disable(disable) => {
            disable.run(&cli).await?;
        }
        Subcommands::Enable(enable) => {
            enable.run(&cli).await?;
        }
        Subcommands::Update(update) => {
            update.run(&cli).await?;
        }
        Subcommands::Modrinth(modrinth) => {
            modrinth.run(&cli).await?;
        }
        Subcommands::Curseforge(curseforge) => {
            curseforge.run(&cli).await?;
        }
        Subcommands::Export(export) => {
            export.run(&cli).await?;
        }
    }

    Ok(())
}
