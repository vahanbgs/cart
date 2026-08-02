use std::collections::{HashSet, VecDeque};

use anyhow::Result;
use cart::api::modrinth;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Modrinth, ModrinthCommand,
    args::modrinth::Add,
    deps::{self, PlanKind, PlannedAdd, WriteData},
    hit_view::HitRow,
    icon_cache::IconCache,
    picker::{self, pick_hit_interactive},
};

impl Modrinth {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            ModrinthCommand::Add(add) => add.run(cli).await,
        }
    }
}

/// Loader string as Modrinth expects for `/v2/project/{slug}/version`
/// resolution — vanilla manifests become the literal `"vanilla"` since
/// that's Modrinth's convention for no-loader compatibility.
fn resolve_loader(config: &Config<'_>) -> &'static str {
    config
        .manifest()
        .loader
        .as_ref()
        .map(|l| l.kind.as_modrinth())
        .unwrap_or("vanilla")
}

/// Loader facet for Modrinth's `/v2/search`. Unlike resolution there is
/// no `"vanilla"` category, so a no-loader manifest maps to `None` and
/// callers drop the facet entirely.
fn search_loader(config: &Config<'_>) -> Option<&'static str> {
    config
        .manifest()
        .loader
        .as_ref()
        .map(|l| l.kind.as_modrinth())
}

struct PendingDep {
    project_id: String,
    kind: PlanKind,
}

async fn perform_add(cli: &Cli, slug: String, disabled: bool, yes: bool) -> anyhow::Result<()> {
    let config = Config::load(cli).await?;
    let path = config.manifest_directory().join("cart.toml");

    let minecraft_version = config.minecraft_version();
    let loader = resolve_loader(&config);

    let http = Client::new();
    let root = modrinth::resolve(&http, &slug, None, minecraft_version, loader).await?;

    let mut document = manifest::load_document(&path).await?;
    let existing_slugs = config.manifest().modrinth_slugs();
    let mut planned_keys = deps::mods_keys(&document);

    let root_key = root.project_slug.clone();
    // Fail fast: skip network-heavy BFS if the target key is already
    // taken. Matches `add_modrinth_mod`'s later guard but avoids doing
    // the transitive dep walk before finding out.
    if planned_keys.contains(&root_key) {
        anyhow::bail!("mod already declared in [mods]: {root_key}");
    }
    planned_keys.insert(root_key.clone());

    let mut plan: Vec<PlannedAdd> = vec![PlannedAdd {
        manifest_key: root_key,
        display_name: root.project_title,
        display_version: root.version_number.clone(),
        kind: PlanKind::Root,
        write: WriteData::Modrinth {
            slug: root.project_slug,
            version: root.version_number,
        },
    }];

    // Visited holds Modrinth project ids (opaque strings) rather than
    // slugs so a mid-flight slug rename can't cause a cycle to be missed.
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(root.project_id);

    let mut queue: VecDeque<PendingDep> = VecDeque::new();
    for dep in root.dependencies {
        enqueue_dep(&mut queue, &mut visited, dep);
    }

    while let Some(pending) = queue.pop_front() {
        // `/v2/project/{id}` accepts both slug and project id, so
        // `resolve()` works unchanged when passed the id here.
        let resolved =
            match modrinth::resolve(&http, &pending.project_id, None, minecraft_version, loader)
                .await
            {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!("skipping modrinth dep {id}: {err}", id = pending.project_id,);
                    continue;
                }
            };

        if existing_slugs.contains(resolved.project_slug.as_str()) {
            tracing::info!(
                "skipping {slug}: already in cart.toml",
                slug = resolved.project_slug,
            );
            continue;
        }
        if planned_keys.contains(&resolved.project_slug) {
            tracing::info!(
                "skipping {slug}: manifest key already in use",
                slug = resolved.project_slug,
            );
            continue;
        }

        planned_keys.insert(resolved.project_slug.clone());
        plan.push(PlannedAdd {
            manifest_key: resolved.project_slug.clone(),
            display_name: resolved.project_title,
            display_version: resolved.version_number.clone(),
            kind: pending.kind,
            write: WriteData::Modrinth {
                slug: resolved.project_slug,
                version: resolved.version_number,
            },
        });

        for dep in resolved.dependencies {
            enqueue_dep(&mut queue, &mut visited, dep);
        }
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
            "added {key} ({title} {ver})",
            key = entry.manifest_key,
            title = entry.display_name,
            ver = entry.display_version,
        );
    }
    manifest::save_document(&path, &document).await?;

    Ok(())
}

fn enqueue_dep(
    queue: &mut VecDeque<PendingDep>,
    visited: &mut HashSet<String>,
    dep: modrinth::VersionDependency,
) {
    let kind = match dep.dependency_type {
        modrinth::DependencyType::Required => PlanKind::Required,
        modrinth::DependencyType::Optional => PlanKind::Optional,
        modrinth::DependencyType::Embedded | modrinth::DependencyType::Incompatible => return,
    };
    let Some(id) = dep.project_id else {
        tracing::warn!("modrinth dep has no project_id; skipping");
        return;
    };
    if visited.insert(id.clone()) {
        queue.push_back(PendingDep {
            project_id: id,
            kind,
        });
    }
}

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Fail fast if there's no manifest to add to — better than
        // spending a picker interaction before finding out.
        let config = Config::load(cli).await?;
        let loader = search_loader(&config);

        let backend = ModrinthBackend {
            http: Client::new(),
            minecraft_version: config.minecraft_version().to_owned(),
            loader,
            installed: config
                .manifest()
                .modrinth_slugs()
                .into_iter()
                .map(|s| s.to_owned())
                .collect(),
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

/// Live-search backend for `mr add`. Owns the HTTP client, MC version,
/// loader facet, and installed-slug set for the whole picker session so
/// each keystroke's fetch can be constructed with no shared borrowing.
struct ModrinthBackend {
    http: Client,
    minecraft_version: String,
    loader: Option<&'static str>,
    installed: HashSet<String>,
}

impl picker::Backend for ModrinthBackend {
    async fn search(&self, query: String, limit: u32) -> Result<Vec<HitRow>> {
        let hits = modrinth::search(
            &self.http,
            &query,
            limit,
            &self.minecraft_version,
            self.loader,
        )
        .await?;
        Ok(hits
            .iter()
            .filter(|h| !self.installed.contains(h.slug.as_str()))
            .map(Into::into)
            .collect())
    }
}
