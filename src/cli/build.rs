use std::path::Path;

use cart::Launcher;
use clap::Args;
use tokio::{fs, io};

use crate::{config::Config, manifest::ModDependency};

use super::Cli;

/// Name of the source directory (next to `cart.toml`) whose contents mirror
/// the `.minecraft/` layout and are replicated into the game directory on
/// every build.
const SOURCE_DIR: &str = "src";

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

        // Replicate `src/` on top of `minecraft/` — always overwrites, so
        // in-place writes by mods (e.g. FML rewriting `fml.toml`) never leak
        // back into the source tree and pack authors' edits always win on
        // the next build.
        copy_source_dir(
            &config.manifest_directory().join(SOURCE_DIR),
            &game_directory,
        )
        .await?;

        Ok(())
    }
}

async fn copy_source_dir(source: &Path, target: &Path) -> anyhow::Result<()> {
    if !fs::try_exists(source).await? {
        return Ok(());
    }

    let mut stack = vec![source.to_owned()];

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            let relative = path.strip_prefix(source)?;
            let destination = target.join(relative);

            if file_type.is_dir() {
                fs::create_dir_all(&destination).await?;
                stack.push(path);
            } else if file_type.is_file() {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).await?;
                }
                // `fs::copy` overwrites the destination even if it's a
                // hardlink from an earlier mod placement.
                fs::copy(&path, &destination).await?;
            }
            // Symlinks are skipped silently.
        }
    }

    Ok(())
}
