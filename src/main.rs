mod cli;
mod config;
mod manifest;

use clap::Parser;

use cart::{Instance, Launcher};

use cli::{Cli, Subcommands};
use config::Config;
use manifest::{Manifest, ModDependency};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match &cli.command {
        Subcommands::Init(init) => {
            init.run(&cli).await?;
        }
        Subcommands::Run => {
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
