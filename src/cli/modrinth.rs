use std::collections::{HashSet, VecDeque};

use cart::api::modrinth;
use reqwest::Client;

use crate::{config::Config, manifest};

use super::{
    Cli, Modrinth, ModrinthCommand,
    args::modrinth::{Add, Find, Search},
    deps::{self, PlanKind, PlannedAdd, WriteData},
    hit_view,
    icon_cache::IconCache,
    picker::pick_hit_tui,
};

impl Modrinth {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match &self.command {
            ModrinthCommand::Add(add) => add.run(cli).await,
            ModrinthCommand::Search(search) => search.run(cli).await,
            ModrinthCommand::Find(find) => find.run(cli).await,
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

impl Add {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let path = config.manifest_directory().join("cart.toml");

        let minecraft_version = config.minecraft_version();
        let loader = resolve_loader(&config);

        let http = Client::new();
        let root = modrinth::resolve(
            &http,
            &self.slug,
            self.version.as_deref(),
            minecraft_version,
            loader,
        )
        .await?;

        let mut document = manifest::load_document(&path).await?;
        let existing_slugs = config.manifest().modrinth_slugs();
        let mut planned_keys = deps::mods_keys(&document);

        let root_key = self
            .name
            .as_deref()
            .unwrap_or(&root.project_slug)
            .to_owned();
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
            let resolved = match modrinth::resolve(
                &http,
                &pending.project_id,
                None,
                minecraft_version,
                loader,
            )
            .await
            {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(
                        "skipping modrinth dep {id}: {err}",
                        id = pending.project_id,
                    );
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
            if !deps::confirm_plan(plan.len() - 1, self.yes).await? {
                plan.truncate(1);
            }
        }

        for entry in &plan {
            let disabled = matches!(entry.kind, PlanKind::Root) && self.disabled;
            entry.apply(&mut document, disabled)?;
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

impl Search {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        // Load the manifest for its Minecraft version + loader — the
        // whole point of `search` is to only surface installable hits,
        // so a missing cart.toml is a hard error, same as `find`.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = search_loader(&config);

        let http = Client::new();
        let mut hits =
            modrinth::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            return Ok(());
        }

        let total = hits.len();
        let installed = config.manifest().modrinth_slugs();
        hits.retain(|h| !installed.contains(h.slug.as_str()));

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
        // Fail fast if there's no manifest to add to — better than
        // spending a network round-trip and a picker interaction
        // before finding out.
        let config = Config::load(cli).await?;
        let minecraft_version = config.minecraft_version();
        let loader = search_loader(&config);

        let http = Client::new();
        let mut hits =
            modrinth::search(&http, &self.query, self.limit, minecraft_version, loader).await?;

        if hits.is_empty() {
            tracing::info!("no results for '{}'", self.query);
            return Ok(());
        }

        let total = hits.len();
        let installed = config.manifest().modrinth_slugs();
        hits.retain(|h| !installed.contains(h.slug.as_str()));

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
        let picked = pick_hit_tui(rows, icons, "Add which mod?").await?;

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
