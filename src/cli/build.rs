use cart::Launcher;
use clap::Args;
use tokio::{fs, io};

use crate::{config::Config, manifest::ModDependency};

use super::Cli;

#[derive(Args)]
pub struct Build;

impl Build {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let launcher = Launcher::new();
        self.run_with(&config, &launcher).await
    }

    pub async fn run_with(&self, config: &Config<'_>, launcher: &Launcher) -> anyhow::Result<()> {
        let cache = launcher.mod_cache();

        let game_directory = config.manifest_directory().join("minecraft/");
        let mods_directory = game_directory.join("mods/");
        fs::create_dir_all(&mods_directory).await?;

        for (mod_name, mod_source) in &config.manifest().mods {
            let source_path = match mod_source {
                ModDependency::Url { url } => cache.fetch_mod(url).await?,
            };

            let target_path = mods_directory.join(format!("{mod_name}.jar"));

            match fs::remove_file(&target_path).await {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => Err(e)?,
            }

            fs::hard_link(source_path, target_path).await?;
        }

        Ok(())
    }
}
