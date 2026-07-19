mod cli;
mod config;
mod manifest;

use clap::Parser;

use cart::Instance;

use cli::{Cli, Commands};
use config::Config;
use manifest::Manifest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let config = Config::load(&cli).await?;

    match config.cli().command {
        Commands::Run => {
            let instance = Instance::builder()
                .version(config.minecraft_version())
                .build(config.manifest_directory().join("minecraft/"));

            instance.launch().await?;
        }
    }

    Ok(())
}
