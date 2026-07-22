use cart::{Instance, Launcher};

use crate::config::Config;

use super::{Cli, Run};

impl Run {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let launcher = Launcher::new();

        self.build.run_with(&config, &launcher).await?;

        let game_directory = config.manifest_directory().join("minecraft/");
        let mut builder = Instance::builder().version(config.minecraft_version());

        if let Some(loader) = config.manifest().loader.clone() {
            builder = builder.loader(loader);
        }

        let instance = builder.build(game_directory);

        launcher.launch(&instance).await
    }
}
