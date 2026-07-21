use std::{collections::HashSet, path::Path};

use anyhow::bail;
use cart::{Launcher, api::modrinth};
use clap::Args;
use reqwest::Client;
use tokio::{fs, io};
use url::Url;

use crate::{
    config::Config,
    manifest::{Manifest, ModDependency},
};

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
        let source_directory = config.manifest_directory().join(SOURCE_DIR);
        fs::create_dir_all(&mods_directory).await?;

        // Manifest is the sole owner of `mods/`. Forbid top-level jars under
        // `src/mods/` — silent skipping would let authors think a dev jar is
        // being placed when it isn't.
        reject_src_mods_jars(&source_directory.join("mods")).await?;

        sync_mods(config, &cache, &mods_directory).await?;

        // Replicate `src/` on top of `minecraft/` — always overwrites, so
        // in-place writes by mods (e.g. FML rewriting `fml.toml`) never leak
        // back into the source tree and pack authors' edits always win on
        // the next build.
        copy_source_dir(&source_directory, &game_directory).await?;

        Ok(())
    }
}

/// Turn a `ModDependency` into the URL the mod-cache should fetch.
///
/// URL entries pass through untouched (they're fully user-pinned). Modrinth
/// entries hit the API to resolve to the concrete file URL — pinned by
/// `version_number` if set, newest compatible otherwise.
async fn resolve_url(
    dep: &ModDependency,
    manifest: &Manifest,
    http: &Client,
) -> anyhow::Result<Url> {
    match dep {
        ModDependency::Url { url, .. } => Ok(url.clone()),
        ModDependency::Modrinth {
            modrinth, version, ..
        } => {
            // TODO: expand when the manifest grows Fabric/NeoForge fields.
            let loader = if manifest.forge.is_some() {
                "forge"
            } else {
                "vanilla"
            };
            let resolved = modrinth::resolve(
                http,
                modrinth,
                version.as_deref(),
                &manifest.minecraft,
                loader,
            )
            .await?;
            Ok(resolved.file.url)
        }
    }
}

async fn sync_mods(
    config: &Config<'_>,
    cache: &cart::ModCache<'_>,
    mods_directory: &Path,
) -> anyhow::Result<()> {
    let expected: HashSet<String> = config
        .manifest()
        .mods
        .iter()
        .map(|(name, entry)| entry.filename(name))
        .collect();

    // Prune any top-level jar the manifest no longer owns. Subdirectories
    // and non-jar files (e.g. `mods/1.12.2/foo.jar` layouts, README files)
    // are left alone.
    let mut entries = fs::read_dir(&mods_directory).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let is_managed = name_str.ends_with(".jar") || name_str.ends_with(".jar.disabled");
        if is_managed && !expected.contains(name_str) {
            fs::remove_file(entry.path()).await?;
        }
    }

    let http = Client::new();
    for (mod_name, mod_source) in &config.manifest().mods {
        let url = resolve_url(mod_source, config.manifest(), &http).await?;
        let cached = tokio::fs::try_exists(cache.path_from_url(&url)?).await?;
        tracing::info!(
            "{action} {mod_name}",
            action = if cached { "cached  " } else { "download" },
        );
        let source_path = cache.fetch_mod(&url).await?;

        let target_path = mods_directory.join(mod_source.filename(mod_name));

        match fs::remove_file(&target_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => Err(e)?,
        }

        fs::hard_link(source_path, target_path).await?;
    }

    Ok(())
}

/// Fail loudly if `src/mods/` contains a top-level jar (enabled or disabled).
/// Subdirectories are fine — those may be version-specific mod folders that
/// belong to individual mods and are none of cart's business.
async fn reject_src_mods_jars(src_mods: &Path) -> anyhow::Result<()> {
    if !fs::try_exists(src_mods).await? {
        return Ok(());
    }

    let mut entries = fs::read_dir(src_mods).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.ends_with(".jar") || name_str.ends_with(".jar.disabled") {
            bail!(
                "src/mods/{name_str} is not allowed — declare it in [mods] in cart.toml instead \
                 (with `disabled = true` if you want it as `.jar.disabled`)"
            );
        }
    }

    Ok(())
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
