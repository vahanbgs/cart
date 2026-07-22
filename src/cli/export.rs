use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use cart::{
    Launcher,
    export::mrpack::{
        self, ModSource, PackEntry, PackFile, PackIndex, ResolvedMod, build_entry,
        dependencies_from,
    },
};
use clap::{Args, ValueEnum};
use reqwest::Client;
use tokio::fs;

use crate::{cli::build, config::Config, manifest::ModDependency};

use super::Cli;

#[derive(Args)]
pub struct Export {
    /// Target format. `mrpack` for Modrinth, `curseforge` for a
    /// CurseForge modpack zip, `prism` for a Prism/MultiMC instance
    /// zip.
    #[arg(value_enum)]
    pub format: Format,

    /// Destination path. Default: `<name>-<version>.<ext>` in the
    /// current directory, where `name` and `version` come from
    /// `cart.toml` and `ext` is picked by format.
    #[arg(short = 'o', long)]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Mrpack,
    Curseforge,
    Prism,
}

impl Format {
    /// File extension the format is conventionally distributed with.
    /// mrpack has its own extension; both CF packs and Prism instances
    /// ship as plain `.zip` — the internals are what distinguishes them.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mrpack => "mrpack",
            Self::Curseforge => "zip",
            Self::Prism => "zip",
        }
    }
}

impl Export {
    pub async fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        let config = Config::load(cli).await?;
        let manifest = config.manifest();

        // All three formats require `name` + `version` — validate up
        // front so each format module doesn't repeat the check.
        let name = manifest
            .name
            .as_deref()
            .context("cart export requires `name` at the top of cart.toml")?;
        let version = manifest
            .version
            .as_deref()
            .context("cart export requires `version` at the top of cart.toml")?;

        let output = self
            .output
            .clone()
            .unwrap_or_else(|| default_output_path(name, version, self.format));

        match self.format {
            Format::Mrpack => {
                let launcher = Launcher::new();
                run_mrpack(&config, &launcher, &output).await?;
                tracing::info!("wrote {}", output.display());
                Ok(())
            }
            Format::Curseforge => {
                bail!(
                    "curseforge export to {} is not yet implemented",
                    output.display()
                )
            }
            Format::Prism => {
                bail!("prism export to {} is not yet implemented", output.display())
            }
        }
    }
}

/// Assemble a `PackIndex` + overrides list from the manifest and write
/// the mrpack ZIP. Every step that talks to the network or the shared
/// mod cache lives here; [`mrpack::write_pack`] itself does only
/// serialization and archive writing.
async fn run_mrpack(
    config: &Config<'_>,
    launcher: &Launcher,
    output: &Path,
) -> anyhow::Result<()> {
    let manifest = config.manifest();
    // `Export::run` already validated `name`/`version` before dispatching
    // here — the `expect` calls document that contract.
    let name = manifest.name.as_deref().expect("name validated by Export::run");
    let version = manifest
        .version
        .as_deref()
        .expect("version validated by Export::run");

    let dependencies = dependencies_from(&manifest.minecraft, manifest.loader.as_ref())?;

    let source_directory = config.manifest_directory().join("src");
    // Same guard `cart build` uses: pack authors sometimes drop dev jars
    // into `src/mods/` expecting them to ship. Silently including them
    // in an mrpack would be worse than a loud error.
    build::reject_src_mods_jars(&source_directory.join("mods")).await?;

    let http = Client::new();
    let curseforge_http = build::build_curseforge_client_if_needed(manifest)?;
    let cache = launcher.mod_cache();

    let mut files: Vec<PackFile> = Vec::new();
    let mut overrides: Vec<(String, PathBuf)> = Vec::new();

    // Sort by manifest key so re-exporting the same manifest produces a
    // byte-identical archive — HashMap iteration order otherwise randomizes
    // both `files[]` ordering and ZIP entry ordering.
    let mut mods: Vec<(&String, &ModDependency)> = manifest.mods.iter().collect();
    mods.sort_by_key(|(name, _)| name.as_str());

    for (mod_name, dep) in mods {
        let url = build::resolve_url(dep, manifest, &http, curseforge_http.as_ref()).await?;
        // `fetch_mod` is idempotent — downloads if the cache misses,
        // returns the path either way. No separate "is cached?" check
        // needed.
        let cached_jar = cache.fetch_mod(&url).await?;
        let filename = dep.filename(mod_name);
        let resolved = ResolvedMod {
            source: source_of(dep),
            filename: &filename,
            cached_jar: &cached_jar,
            download_url: Some(url.as_str()),
        };
        match build_entry(&resolved)? {
            PackEntry::File(f) => files.push(f),
            PackEntry::Override { source, dest } => overrides.push((dest, source)),
        }
    }

    collect_src_overrides(&source_directory, &mut overrides).await?;
    // Deterministic archive ordering, same reason as the `mods` sort above.
    overrides.sort_by(|a, b| a.0.cmp(&b.0));

    let index = PackIndex {
        format_version: 1,
        game: "minecraft".to_owned(),
        version_id: version.to_owned(),
        name: name.to_owned(),
        summary: manifest.summary.clone(),
        files,
        dependencies,
    };

    mrpack::write_pack(&index, &overrides, output)
}

fn source_of(dep: &ModDependency) -> ModSource {
    match dep {
        ModDependency::Modrinth { .. } => ModSource::Modrinth,
        ModDependency::CurseForge { .. } => ModSource::CurseForge,
        ModDependency::Url { .. } => ModSource::Url,
    }
}

/// Walk `source_directory` (typically `<manifest_dir>/src/`) and append
/// every file as `("overrides/<relative>", absolute_path)`. Directories
/// are recursed; symlinks are skipped silently (same as `copy_source_dir`
/// in `cart build`). No-op if the directory doesn't exist — a pack that
/// only ships mods is still a valid mrpack.
async fn collect_src_overrides(
    source_directory: &Path,
    into: &mut Vec<(String, PathBuf)>,
) -> anyhow::Result<()> {
    if !fs::try_exists(source_directory).await? {
        return Ok(());
    }

    let mut stack = vec![source_directory.to_owned()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let relative = path.strip_prefix(source_directory)?;
                let relative_str = relative.to_str().with_context(|| {
                    format!("src/ contains a non-UTF-8 path: {}", relative.display())
                })?;
                // ZIP entries always use `/` separators regardless of host OS.
                let dest = format!("overrides/{}", relative_str.replace('\\', "/"));
                into.push((dest, path));
            }
        }
    }

    Ok(())
}

/// `<name>-<version>.<ext>` in the current directory. Same shape every
/// format uses, so extract once — the export modules land on top of
/// this helper rather than recomputing per format.
fn default_output_path(name: &str, version: &str, format: Format) -> PathBuf {
    PathBuf::from(format!("{name}-{version}.{}", format.extension()))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    #[test]
    fn source_of_maps_each_variant() {
        let modrinth = ModDependency::Modrinth {
            modrinth: "jei".to_owned(),
            version: None,
            disabled: false,
        };
        let curseforge = ModDependency::CurseForge {
            curseforge: 1,
            file: 2,
            disabled: false,
        };
        let url = ModDependency::Url {
            url: Url::parse("https://example.com/x.jar").unwrap(),
            disabled: false,
        };
        assert_eq!(source_of(&modrinth), ModSource::Modrinth);
        assert_eq!(source_of(&curseforge), ModSource::CurseForge);
        assert_eq!(source_of(&url), ModSource::Url);
    }

    /// Two invariants for the src/ walk: relative paths flatten under
    /// `overrides/` with `/` separators, and a missing `src/` is a no-op
    /// rather than an error (mod-only packs are legal mrpacks).
    #[tokio::test]
    async fn collect_src_overrides_prefixes_and_recurses() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(source.join("config/sub")).await.unwrap();
        fs::write(source.join("options.txt"), b"a").await.unwrap();
        fs::write(source.join("config/foo.toml"), b"b").await.unwrap();
        fs::write(source.join("config/sub/bar.json"), b"c")
            .await
            .unwrap();

        let mut overrides = Vec::new();
        collect_src_overrides(&source, &mut overrides).await.unwrap();

        let mut dests: Vec<&str> = overrides.iter().map(|(d, _)| d.as_str()).collect();
        dests.sort();
        assert_eq!(
            dests,
            vec![
                "overrides/config/foo.toml",
                "overrides/config/sub/bar.json",
                "overrides/options.txt",
            ]
        );
    }

    #[tokio::test]
    async fn collect_src_overrides_missing_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let mut overrides = Vec::new();
        collect_src_overrides(&dir.path().join("src"), &mut overrides)
            .await
            .unwrap();
        assert!(overrides.is_empty());
    }

    #[test]
    fn default_output_path_mrpack() {
        assert_eq!(
            default_output_path("my-pack", "1.0.0", Format::Mrpack),
            PathBuf::from("my-pack-1.0.0.mrpack")
        );
    }

    #[test]
    fn default_output_path_curseforge() {
        assert_eq!(
            default_output_path("my-pack", "1.0.0", Format::Curseforge),
            PathBuf::from("my-pack-1.0.0.zip")
        );
    }

    #[test]
    fn default_output_path_prism() {
        assert_eq!(
            default_output_path("my-pack", "1.0.0", Format::Prism),
            PathBuf::from("my-pack-1.0.0.zip")
        );
    }
}
