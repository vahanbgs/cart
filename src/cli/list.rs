use clap::Args;

use crate::{config::Config, manifest::ModDependency};

use super::Cli;

#[derive(Args)]
pub struct List;

impl List {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;

        let mut entries: Vec<(&String, &ModDependency)> =
            config.manifest().mods.iter().collect();
        entries.sort_by_key(|(name, _)| name.as_str());

        if entries.is_empty() {
            return Ok(());
        }

        let name_width = entries.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        let source_width = entries
            .iter()
            .map(|(_, d)| source_label(d).len())
            .max()
            .unwrap_or(0);
        let version_width = entries
            .iter()
            .map(|(_, d)| version_label(d).len())
            .max()
            .unwrap_or(0);

        for (name, dep) in entries {
            let disabled_suffix = if dep.is_disabled() { "  (disabled)" } else { "" };
            let line = format!(
                "{name:<name_width$}  {source:<source_width$}  {version:<version_width$}{disabled_suffix}",
                source = source_label(dep),
                version = version_label(dep),
            );
            println!("{}", line.trim_end());
        }

        Ok(())
    }
}

fn source_label(dep: &ModDependency) -> &'static str {
    match dep {
        ModDependency::Modrinth { .. } => "modrinth",
        ModDependency::Url { .. } => "url",
    }
}

/// Text for the version column. Modrinth entries show their pinned
/// `version_number`, or `(unpinned)` if loose — hinting that the actual
/// version is decided at build time. URL entries have no meaningful
/// version to show (the URL is the pin).
fn version_label(dep: &ModDependency) -> String {
    match dep {
        ModDependency::Modrinth {
            version: Some(v), ..
        } => v.clone(),
        ModDependency::Modrinth { version: None, .. } => String::from("(unpinned)"),
        ModDependency::Url { .. } => String::new(),
    }
}
