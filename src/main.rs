mod cli;
mod config;
mod manifest;

use clap::Parser;

use cart::{Instance, Launcher};

use cli::{Cli, Subcommands};
use config::Config;
use manifest::{Manifest, ModDependency};
use tokio::{fs, io};

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

            let game_directory = config.manifest_directory().join("minecraft/");
            let mods_directory = game_directory.join("mods/");
            fs::create_dir_all(&mods_directory).await?;

            for (mod_name, mod_source) in &config.manifest().mods {
                let source_path = match mod_source {
                    ModDependency::Url { url } => cache.fetch_mod(&url).await?,
                };

                let target_path = mods_directory.join(format!("{mod_name}.jar"));

                match fs::remove_file(&target_path).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => Err(e)?,
                }

                fs::hard_link(source_path, target_path).await?;
            }

            let instance = Instance::builder()
                .version(config.minecraft_version())
                .build(game_directory);

            launcher.launch(&instance).await?;
        }
    }

    Ok(())
}
