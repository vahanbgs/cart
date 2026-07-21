use anyhow::anyhow;
use cart::api::modrinth;
use clap::Args;
use reqwest::Client;

use crate::{config::Config, manifest, manifest::ModDependency};

use super::Cli;

#[derive(Args)]
pub struct Update {
    /// Update only these mods. If empty, updates every Modrinth-sourced
    /// entry in the manifest.
    pub names: Vec<String>,
}

impl Update {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");
        let minecraft_version = &config.manifest().minecraft;
        // TODO: expand when the manifest grows Fabric/NeoForge fields.
        let loader = if config.manifest().forge.is_some() {
            "forge"
        } else {
            "vanilla"
        };

        // Named subset validation — fail fast on typos rather than silently
        // updating nothing.
        let mut targets: Vec<(&String, &ModDependency)> = if self.names.is_empty() {
            config.manifest().mods.iter().collect()
        } else {
            let mut out = Vec::new();
            for name in &self.names {
                let dep = config
                    .manifest()
                    .mods
                    .get(name)
                    .ok_or_else(|| anyhow!("mod not found in [mods]: {name}"))?;
                out.push((name, dep));
            }
            out
        };
        targets.sort_by_key(|(n, _)| n.as_str());

        let http = Client::new();
        let mut document = manifest::load_document(&path).await?;
        let mut any_change = false;

        for (name, dep) in targets {
            let (slug, current) = match dep {
                ModDependency::Modrinth {
                    modrinth, version, ..
                } => (modrinth, version.as_deref()),
                ModDependency::Url { .. } => {
                    tracing::debug!("{name}: url entry, skipped");
                    continue;
                }
                ModDependency::CurseForge { .. } => {
                    tracing::debug!("{name}: curseforge entry, update not yet implemented");
                    continue;
                }
            };

            // Per-entry errors (Modrinth 5xx, project renamed, no compatible
            // version after an mc bump) shouldn't block the rest of the run
            // — partial updates beat all-or-nothing.
            let resolved =
                match modrinth::resolve(&http, slug, None, minecraft_version, loader).await {
                    Ok(r) => r,
                    Err(err) => {
                        tracing::warn!("{name}: {err:#}");
                        continue;
                    }
                };

            let new_version = &resolved.version_number;
            match current {
                Some(cur) if cur == new_version => {
                    tracing::info!("{name}: up to date ({cur})");
                }
                Some(cur) => {
                    tracing::info!("{name}: {cur} → {new_version}");
                    manifest::set_mod_version(&mut document, name, new_version)?;
                    any_change = true;
                }
                None => {
                    tracing::info!("{name}: pinned to {new_version}");
                    manifest::set_mod_version(&mut document, name, new_version)?;
                    any_change = true;
                }
            }
        }

        if any_change {
            manifest::save_document(&path, &document).await?;
        }

        Ok(())
    }
}
