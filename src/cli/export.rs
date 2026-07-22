use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Args, ValueEnum};

use crate::config::Config;

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
                bail!("mrpack export to {} is not yet implemented", output.display())
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

/// `<name>-<version>.<ext>` in the current directory. Same shape every
/// format uses, so extract once — the export modules land on top of
/// this helper rather than recomputing per format.
fn default_output_path(name: &str, version: &str, format: Format) -> PathBuf {
    PathBuf::from(format!("{name}-{version}.{}", format.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
