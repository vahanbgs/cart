use tokio::fs;

use super::{Cli, Init};

impl Init {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        fs::create_dir_all(&self.path).await?;
        super::scaffold::scaffold(cli, &self.path).await
    }
}
