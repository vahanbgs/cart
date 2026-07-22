use anyhow::{Context, anyhow};
use cart::api::{curseforge, modrinth};
use reqwest::Client;
use toml_edit::DocumentMut;

use crate::{config::Config, manifest, manifest::ModDependency};

use super::{Cli, Update};

/// Env var holding the CurseForge API key. Only read if the manifest
/// has at least one CurseForge entry among the update targets.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

impl Update {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");
        let minecraft_version = &config.manifest().minecraft;
        let loader_kind = config.manifest().loader.as_ref().map(|l| l.kind);
        let modrinth_loader = match loader_kind {
            Some(cart::LoaderKind::Fabric) => "fabric",
            Some(cart::LoaderKind::Forge) => "forge",
            Some(cart::LoaderKind::NeoForge) => "neoforge",
            None => "vanilla",
        };
        let curseforge_loader = loader_kind.map(|k| match k {
            cart::LoaderKind::Fabric => curseforge::LoaderType::Fabric,
            cart::LoaderKind::Forge => curseforge::LoaderType::Forge,
            cart::LoaderKind::NeoForge => curseforge::LoaderType::NeoForge,
        });

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
        let curseforge_http = build_curseforge_client_if_needed(&targets)?;

        let mut document = manifest::load_document(&path).await?;
        let mut any_change = false;

        for (name, dep) in targets {
            let changed = match dep {
                ModDependency::Modrinth {
                    modrinth: slug,
                    version,
                    ..
                } => {
                    update_modrinth(
                        &http,
                        &mut document,
                        name,
                        slug,
                        version.as_deref(),
                        minecraft_version,
                        modrinth_loader,
                    )
                    .await
                }
                ModDependency::CurseForge {
                    curseforge: project_id,
                    file,
                    ..
                } => {
                    let cf = curseforge_http
                        .as_ref()
                        .expect("curseforge client should have been built");
                    update_curseforge(
                        cf,
                        &mut document,
                        name,
                        *project_id,
                        *file,
                        minecraft_version,
                        curseforge_loader,
                    )
                    .await
                }
                ModDependency::Url { .. } => {
                    tracing::debug!("{name}: url entry, skipped");
                    Ok(false)
                }
            };

            match changed {
                Ok(c) => any_change |= c,
                // Per-entry errors (5xx, project renamed, no compatible
                // version after an mc bump) shouldn't block the rest of
                // the run — partial updates beat all-or-nothing.
                Err(err) => tracing::warn!("{name}: {err:#}"),
            }
        }

        if any_change {
            manifest::save_document(&path, &document).await?;
        }

        Ok(())
    }
}

fn build_curseforge_client_if_needed(
    targets: &[(&String, &ModDependency)],
) -> anyhow::Result<Option<Client>> {
    let has_curseforge = targets
        .iter()
        .any(|(_, d)| matches!(d, ModDependency::CurseForge { .. }));
    if !has_curseforge {
        return Ok(None);
    }
    let key = std::env::var(CURSEFORGE_API_KEY_ENV).with_context(|| {
        format!(
            "cart update on a CurseForge entry needs {CURSEFORGE_API_KEY_ENV} — get a key at \
             https://console.curseforge.com/ and export it"
        )
    })?;
    Ok(Some(curseforge::client(&key)?))
}

async fn update_modrinth(
    http: &Client,
    document: &mut DocumentMut,
    name: &str,
    slug: &str,
    current: Option<&str>,
    minecraft_version: &str,
    loader: &str,
) -> anyhow::Result<bool> {
    let resolved = modrinth::resolve(http, slug, None, minecraft_version, loader).await?;
    let new_version = &resolved.version_number;
    match current {
        Some(cur) if cur == new_version => {
            tracing::info!("{name}: up to date ({cur})");
            Ok(false)
        }
        Some(cur) => {
            tracing::info!("{name}: {cur} → {new_version}");
            manifest::set_mod_version(document, name, new_version)?;
            Ok(true)
        }
        None => {
            tracing::info!("{name}: pinned to {new_version}");
            manifest::set_mod_version(document, name, new_version)?;
            Ok(true)
        }
    }
}

async fn update_curseforge(
    http: &Client,
    document: &mut DocumentMut,
    name: &str,
    project_id: u32,
    current_file_id: u32,
    minecraft_version: &str,
    loader: Option<curseforge::LoaderType>,
) -> anyhow::Result<bool> {
    let latest = curseforge::latest_file(http, project_id, minecraft_version, loader).await?;
    if latest.id == current_file_id {
        tracing::info!("{name}: up to date (file {current_file_id})");
        Ok(false)
    } else {
        tracing::info!(
            "{name}: file {current_file_id} → {new_id}",
            new_id = latest.id
        );
        manifest::set_mod_file(document, name, latest.id)?;
        Ok(true)
    }
}
