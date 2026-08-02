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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Subcommands};

    #[tokio::test]
    async fn new_errors_when_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli {
            directory: None,
            manifest_path: None,
            verbose: 0,
            minecraft_version: None,
            command: Subcommands::New(New {
                path: dir.path().to_path_buf(),
            }),
        };
        let Subcommands::New(new) = &cli.command else {
            unreachable!()
        };
        let err = new.run(&cli).await.unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
    }
}
