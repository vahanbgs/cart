use std::collections::HashSet;
use std::io::IsTerminal;

use inquire::Confirm;
use toml_edit::{DocumentMut, Item};

use crate::manifest;

/// Shared vocabulary for the Modrinth and CurseForge dep-resolution
/// paths. Kept source-neutral so both call sites format the plan and
/// prompt identically.
#[derive(Clone, Copy, Debug)]
pub enum PlanKind {
    Root,
    Required,
    Optional,
}

pub struct PlannedAdd {
    pub manifest_key: String,
    pub display_name: String,
    pub display_version: String,
    pub kind: PlanKind,
    pub write: WriteData,
}

/// Source-specific fields needed to actually stamp the entry into
/// `cart.toml`. The enum lives here (not in each CLI file) so `apply`
/// can dispatch once at the end of the BFS.
pub enum WriteData {
    Modrinth { slug: String, version: String },
    CurseForge { project_id: u32, file_id: u32 },
}

impl PlannedAdd {
    pub fn apply(&self, document: &mut DocumentMut, disabled: bool) -> anyhow::Result<()> {
        match &self.write {
            WriteData::Modrinth { slug, version } => manifest::add_modrinth_mod(
                document,
                &self.manifest_key,
                slug,
                version,
                disabled,
            ),
            WriteData::CurseForge {
                project_id,
                file_id,
            } => manifest::add_curseforge_mod(
                document,
                &self.manifest_key,
                *project_id,
                *file_id,
                disabled,
            ),
        }
    }
}

/// Snapshot of the current `[mods]` keys. Callers use it to detect
/// collisions before the source-specific `add_*_mod` helper would bail,
/// which lets deps be skipped cleanly instead of erroring the whole op.
pub fn mods_keys(document: &DocumentMut) -> HashSet<String> {
    document
        .get("mods")
        .and_then(Item::as_table)
        .map(|t| t.iter().map(|(k, _)| k.to_owned()).collect())
        .unwrap_or_default()
}

pub fn print_plan(plan: &[PlannedAdd]) {
    println!("Will add:");
    let name_w = plan.iter().map(|p| p.display_name.len()).max().unwrap_or(0);
    let key_w = plan.iter().map(|p| p.manifest_key.len()).max().unwrap_or(0);
    for p in plan {
        let tag = match p.kind {
            PlanKind::Root => "(root)  ",
            PlanKind::Required => "required",
            PlanKind::Optional => "optional",
        };
        println!(
            "  {tag}  {name:name_w$}  {key:key_w$}  {ver}",
            name = p.display_name,
            key = p.manifest_key,
            ver = p.display_version,
        );
    }
}

/// `--yes` and a non-tty stdin both short-circuit to accept, so
/// `cart add` stays scriptable via pipes and CI.
pub async fn confirm_plan(dep_count: usize, yes: bool) -> anyhow::Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        tracing::info!("stdin is not a tty; auto-accepting {dep_count} dep(s)");
        return Ok(true);
    }
    let prompt = format!(
        "Add {dep_count} dependenc{} to cart.toml?",
        if dep_count == 1 { "y" } else { "ies" },
    );
    let accepted = tokio::task::spawn_blocking(move || {
        Confirm::new(&prompt).with_default(true).prompt()
    })
    .await??;
    Ok(accepted)
}
