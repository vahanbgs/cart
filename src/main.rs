mod cli;
mod config;
mod manifest;

use clap::Parser;
use tokio::fs;

use cart::{Instance, Launcher};

use cli::{Cli, Commands};
use config::Config;
use manifest::{Manifest, ModDependency};

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

            let launcher = Launcher::new();
            let cache = launcher.mod_cache();

            for (_mod_name, mod_source) in &config.manifest().mods {
                let _path = match mod_source {
                    ModDependency::Url { url } => cache.fetch_mod(&url).await?,
                };
            }

            let instance = Instance::builder()
                .version(config.minecraft_version())
                .build(config.manifest_directory().join("minecraft/"));

            launcher.launch(&instance).await?;
        }
    }

    Ok(())
}
