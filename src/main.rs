mod cli;
mod config;
mod manifest;

use clap::Parser;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use cli::{Cli, Subcommands};
use manifest::Manifest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // `minecraft.stdout`/`minecraft.stderr` are trace-level events
    // emitted by the launcher forwarding the game's fd 1/2. Silenced
    // by default; `-vv` opts them into the terminal.
    let default_filter = match cli.verbose {
        0 => "cart=info,minecraft=off",
        1 => "cart=debug,minecraft=off",
        _ => "cart=trace,minecraft=trace",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    // `IndicatifLayer` owns a `MultiProgress` and provides a writer that
    // suspends bar rendering around each write, so `tracing::info!`/etc.
    // interleave cleanly above active bars.
    let indicatif_layer = IndicatifLayer::new();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(indicatif_layer.get_stderr_writer())
        .without_time()
        .with_target(false)
        .compact();

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(indicatif_layer)
        .init();

    match &cli.command {
        Subcommands::Build(build) => {
            build.run(&cli).await?;
        }
        Subcommands::Init(init) => {
            init.run(&cli).await?;
        }
        Subcommands::New(new) => {
            new.run(&cli).await?;
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
