use std::collections::{HashSet, VecDeque};

use anyhow::Context;
use cart::api::curseforge;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Curseforge, CurseforgeCommand,
    args::curseforge::{Add, Find, Search},
    deps::{self, PlanKind, PlannedAdd, WriteData},
    hit_view,
    icon_cache::IconCache,
    picker::pick_hit_tui,
};

/// Env var holding the CurseForge API key. Only read when a CurseForge
/// subcommand actually runs — the Modrinth tree never touches it.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

impl Curseforge {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            CurseforgeCommand::Add(add) => add.run(cli).await,
            CurseforgeCommand::Search(search) => search.run(cli).await,
            CurseforgeCommand::Find(find) => find.run(cli).await,
        }
    }
}

/// `CURSEFORGE_API_KEY` guarded read used by every CF-hitting subcommand.
/// Same error text everywhere so users see one consistent hint.
fn cf_client() -> anyhow::Result<Client> {
    let key = std::env::var(CURSEFORGE_API_KEY_ENV).with_context(|| {
        format!(
            "cart curseforge subcommands need {CURSEFORGE_API_KEY_ENV} — get a key at \
             https://console.curseforge.com/ and export it"
        )
    })?;
    curseforge::client(&key)
}

/// Loader for CurseForge's `modLoaderType` filter. Vanilla manifests
/// return `None`; callers pass that straight through and CF drops the
/// param, which is what we want.
fn cf_loader(config: &Config<'_>) -> Option<curseforge::LoaderType> {
    config.manifest().loader.as_ref().map(|l| l.kind.into())
}

struct PendingDep {
    project_id: u32,
    kind: PlanKind,
}

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let minecraft_version = config.minecraft_version();
        let loader = cf_loader(&config);

        let http = cf_client()?;

        let project = curseforge::find_project_by_slug(&http, &self.slug).await?;
        let file = if let Some(pin) = self.version.as_deref() {
            let file_id: u32 = pin.parse().with_context(|| {
                format!("curseforge --version must be a numeric file id (got '{pin}')")
            })?;
            curseforge::fetch_file(&http, project.id, file_id).await?
        } else {
            curseforge::latest_file(&http, project.id, minecraft_version, loader).await?
        };

        let mut document = manifest::load_document(&path).await?;
        let existing_ids = config.manifest().curseforge_project_ids();
        let mut planned_keys = deps::mods_keys(&document);

        let root_key = self.name.as_deref().unwrap_or(&project.slug).to_owned();
        if planned_keys.contains(&root_key) {
            anyhow::bail!("mod already declared in [mods]: {root_key}");
        }
        planned_keys.insert(root_key.clone());

        let mut plan: Vec<PlannedAdd> = vec![PlannedAdd {
            manifest_key: root_key,
            display_name: project.name.clone(),
            display_version: file.file_name.clone(),
            kind: PlanKind::Root,
            write: WriteData::CurseForge {
                project_id: project.id,
                file_id: file.id,
            },
        }];

        let mut visited: HashSet<u32> = HashSet::new();
        visited.insert(project.id);

        let mut queue: VecDeque<PendingDep> = VecDeque::new();
        for dep in &file.dependencies {
            enqueue_dep(&mut queue, &mut visited, dep);
        }

        while let Some(pending) = queue.pop_front() {
            let dep_project = match curseforge::get_project(&http, pending.project_id).await {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(
                        "skipping curseforge dep {id}: {err}",
                        id = pending.project_id,
                    );
                    continue;
                }
            };
            if existing_ids.contains(&dep_project.id) {
                tracing::info!(
                    "skipping {slug}: already in cart.toml",
                    slug = dep_project.slug,
                );
                continue;
            }
            if planned_keys.contains(&dep_project.slug) {
                tracing::info!(
                    "skipping {slug}: manifest key already in use",
                    slug = dep_project.slug,
                );
                continue;
            }
            let dep_file = match curseforge::latest_file(
                &http,
                dep_project.id,
                minecraft_version,
                loader,
            )
            .await
            {
                Ok(f) => f,
                Err(err) => {
                    tracing::warn!("skipping {slug}: {err}", slug = dep_project.slug);
                    continue;
                }
            };

            planned_keys.insert(dep_project.slug.clone());
            for d in &dep_file.dependencies {
                enqueue_dep(&mut queue, &mut visited, d);
            }
            plan.push(PlannedAdd {
                manifest_key: dep_project.slug,
                display_name: dep_project.name,
                display_version: dep_file.file_name,
                kind: pending.kind,
                write: WriteData::CurseForge {
                    project_id: dep_project.id,
                    file_id: dep_file.id,
                },
            });
        }

        if plan.len() > 1 {
            deps::print_plan(&plan);
            if !deps::confirm_plan(plan.len() - 1, self.yes).await? {
                plan.truncate(1);
            }
        }

        for entry in &plan {
            let disabled = matches!(entry.kind, PlanKind::Root) && self.disabled;
            entry.apply(&mut document, disabled)?;
            tracing::info!(
                "added {key} ({title}, {filename})",
                key = entry.manifest_key,
                title = entry.display_name,
                filename = entry.display_version,
            );
        }
        manifest::save_document(&path, &document).await?;

        Ok(())
    }
}

fn enqueue_dep(
    queue: &mut VecDeque<PendingDep>,
    visited: &mut HashSet<u32>,
    dep: &curseforge::FileDependency,
) {
    let kind = if dep.relation_type == curseforge::RelationType::RequiredDependency as u8 {
        PlanKind::Required
    } else if dep.relation_type == curseforge::RelationType::OptionalDependency as u8 {
        PlanKind::Optional
    } else {
        return;
    };
    // CF has been observed to emit mod_id == 0 for internal/aux relations.
    if dep.mod_id == 0 {
        return;
    }
    if visited.insert(dep.mod_id) {
        queue.push_back(PendingDep {
            project_id: dep.mod_id,
            kind,
        });
    }
}

impl Search {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Same story as `find`: without a manifest we can't filter, and
        // the point of `search` is to only show installable hits.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = cf_loader(&config);

        let http = cf_client()?;
        let mut hits =
            curseforge::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            return Ok(());
        }

        let total = hits.len();
        let installed = config.manifest().curseforge_project_ids();
        hits.retain(|h| !installed.contains(&h.id));

        if hits.is_empty() {
            tracing::info!("all {total} result(s) already in cart.toml");
            return Ok(());
        }

        let hidden = total - hits.len();
        if hidden > 0 {
            hit_view::print_hidden_note(hidden, total);
        }

        let rows: Vec<hit_view::HitRow> = hits.iter().map(Into::into).collect();
        let cache = IconCache::shared()?;
        hit_view::print_search_results(&rows, &cache).await;

        Ok(())
    }
}

impl Find {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Fail fast on a missing manifest before touching the network
        // or the picker.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = cf_loader(&config);

        let http = cf_client()?;
        let mut hits =
            curseforge::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            tracing::info!("no results for '{}'", self.query);
            return Ok(());
        }

        let total = hits.len();
        let installed = config.manifest().curseforge_project_ids();
        hits.retain(|h| !installed.contains(&h.id));

        if hits.is_empty() {
            tracing::info!("all {total} result(s) already in cart.toml");
            return Ok(());
        }

        let hidden = total - hits.len();
        if hidden > 0 {
            hit_view::print_hidden_note(hidden, total);
        }

        let rows: Vec<hit_view::HitRow> = hits.iter().map(Into::into).collect();
        let cache = IconCache::shared()?;
        let icons = hit_view::prefetch_icons(&cache, &rows).await;
        let Some(picked) = pick_hit_tui(rows, icons, "Add which mod?").await? else {
            return Ok(());
        };

        let add = Add {
            slug: picked.slug,
            version: None,
            name: None,
            disabled: self.disabled,
            yes: self.yes,
        };
        add.run(cli).await
    }
}
