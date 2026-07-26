use tokio::fs;

use super::{Cli, New};

impl New {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Refuse to touch an existing path — matches `cargo new`. Users
        // who want to initialize a directory they've already created
        // should reach for `cart init` instead.
        if fs::try_exists(&self.path).await? {
            anyhow::bail!("{} already exists", self.path.display());
        }
        fs::create_dir_all(&self.path).await?;
        super::scaffold::scaffold(cli, &self.path).await
    }
}
