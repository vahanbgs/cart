use clap::Args;

use crate::config::Config;

use super::Cli;

#[derive(Args)]
pub struct List;

impl List {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;

        let mut names: Vec<&String> = config.manifest().mods.keys().collect();
        names.sort();

        for name in names {
            let entry = &config.manifest().mods[name];
            if entry.is_disabled() {
                println!("{name} (disabled)");
            } else {
                println!("{name}");
            }
        }

        Ok(())
    }
}
