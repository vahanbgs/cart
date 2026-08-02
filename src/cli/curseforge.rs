use std::collections::{HashSet, VecDeque};

use anyhow::{Context, Result};
use cart::api::curseforge;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Curseforge, CurseforgeCommand,
    args::curseforge::Add,
    deps::{self, PlanKind, PlannedAdd, WriteData},
    hit_view::HitRow,
    icon_cache::IconCache,
    picker::{self, pick_hit_interactive},
};

/// Env var holding the CurseForge API key. Only read when a CurseForge
/// subcommand actually runs — the Modrinth tree never touches it.
const CURSEFORGE_API_KEY_ENV: &str = "CURSEFORGE_API_KEY";

impl Curseforge {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            CurseforgeCommand::Add(add) => add.run(cli).await,
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

async fn perform_add(cli: &Cli, slug: String, disabled: bool, yes: bool) -> anyhow::Result<()> {
    let config = Config::load(cli).await?;
    let path = config.manifest_directory().join("cart.toml");

    let minecraft_version = config.minecraft_version();
    let loader = cf_loader(&config);

    let http = cf_client()?;

    let project = curseforge::find_project_by_slug(&http, &slug).await?;
    let file = curseforge::latest_file(&http, project.id, minecraft_version, loader).await?;

    let mut document = manifest::load_document(&path).await?;
    let existing_ids = config.manifest().curseforge_project_ids();
    let mut planned_keys = deps::mods_keys(&document);

    let root_key = project.slug.clone();
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
        let dep_file =
            match curseforge::latest_file(&http, dep_project.id, minecraft_version, loader).await {
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
        if !deps::confirm_plan(plan.len() - 1, yes).await? {
            plan.truncate(1);
        }
    }

    for entry in &plan {
        let entry_disabled = matches!(entry.kind, PlanKind::Root) && disabled;
        entry.apply(&mut document, entry_disabled)?;
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

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Fail fast on a missing manifest or missing API key before
        // opening the picker.
        let config = Config::load(cli).await?;
        let loader = cf_loader(&config);
        let http = cf_client()?;

        let backend = CurseforgeBackend {
            http,
            minecraft_version: config.minecraft_version().to_owned(),
            loader,
            installed: config.manifest().curseforge_project_ids(),
        };
        let icon_cache = IconCache::shared()?;
        let initial_query = self.query.clone().unwrap_or_default();

        let picks = pick_hit_interactive(
            backend,
            icon_cache,
            initial_query,
            self.limit,
            "Add which mod?",
        )
        .await?;
        if picks.is_empty() {
            return Ok(());
        }

        for picked in picks {
            perform_add(cli, picked.slug, self.disabled, self.yes).await?;
        }
        Ok(())
    }
}

/// Live-search backend for `cf add`. Owns everything the fetch loop
/// needs (HTTP client with API key baked in, MC version, loader facet,
/// installed project ids) so per-keystroke searches don't borrow across
/// task boundaries.
struct CurseforgeBackend {
    http: Client,
    minecraft_version: String,
    loader: Option<curseforge::LoaderType>,
    installed: HashSet<u32>,
}

impl picker::Backend for CurseforgeBackend {
    async fn search(&self, query: String, limit: u32) -> Result<Vec<HitRow>> {
        let hits = curseforge::search(
            &self.http,
            &query,
            limit,
            &self.minecraft_version,
            self.loader,
        )
        .await?;
        Ok(hits
            .iter()
            .filter(|h| !self.installed.contains(&h.id))
            .map(Into::into)
            .collect())
    }
}
