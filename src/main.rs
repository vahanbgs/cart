mod cli;
mod config;
mod manifest;

use clap::Parser;
use tokio::fs;

use cart::Instance;

use cli::{Cli, Commands};
use config::Config;
use manifest::Manifest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Init { path } => {
            fs::create_dir_all(path).await?;
            fs::write(
                path.join("cart.toml"),
                toml::to_string_pretty(&Manifest::new(&cli))?,
            )
            .await?;
        }
        Commands::Run => {
            let config = Config::load(&cli).await?;

            let instance = Instance::builder()
                .version(config.minecraft_version())
                .build(config.manifest_directory().join("minecraft/"));

            instance.launch().await?;
        }
    }

    Ok(())
}
