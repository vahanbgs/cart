use anyhow::Context;
use tokio::fs;

use super::{Cli, Init};

impl Init {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Dir must already exist — cart init does not create it. That's
        // `cart new`'s job; keeping the two commands non-overlapping is
        // the whole point of the split.
        let meta = fs::metadata(&self.path).await.with_context(|| {
            format!(
                "{} does not exist — use `cart new` to create it",
                self.path.display()
            )
        })?;
        anyhow::ensure!(meta.is_dir(), "{} is not a directory", self.path.display());

        // Refuse to clobber an existing pack.
        let manifest_path = self.path.join("cart.toml");
        if fs::try_exists(&manifest_path).await? {
            anyhow::bail!("{} already exists", manifest_path.display());
        }

        super::scaffold::scaffold(cli, &self.path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Subcommands};

    fn cli_for(path: std::path::PathBuf) -> Cli {
        Cli {
            directory: None,
            manifest_path: None,
            verbose: 0,
            minecraft_version: None,
            command: Subcommands::Init(Init { path }),
        }
    }

    #[tokio::test]
    async fn init_errors_when_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let cli = cli_for(missing.clone());
        let Subcommands::Init(init) = &cli.command else {
            unreachable!()
        };
        let err = init.run(&cli).await.unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn init_errors_when_manifest_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cart.toml"), "").unwrap();
        let cli = cli_for(dir.path().to_path_buf());
        let Subcommands::Init(init) = &cli.command else {
            unreachable!()
        };
        let err = init.run(&cli).await.unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
    }
}
