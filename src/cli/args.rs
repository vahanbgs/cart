//! All clap-derive type definitions for cart's CLI, in one place.
//!
//! This file is the single source of truth for the command-line shape:
//! `main.rs` reaches these via the regular `mod cli;` graph, and
//! `build.rs` reaches them via `#[path = "src/cli/args.rs"] mod args;`
//! so shell completions are generated from the same derive that runtime
//! parsing uses.
//!
//! Nothing here may `use crate::*` — `build.rs` compiles this file
//! before the rest of the crate exists. Only `std`, `clap`, and
//! internal submodules are allowed. Impl blocks (the async `run`
//! methods that talk to Config/Manifest/HTTP) live in the sibling
//! per-command files.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
pub struct Cli {
    /// Run as if invoked from this directory. The cart project's `cart.toml`
    /// is expected to live directly inside it. Mirrors `make -C` / `cargo -C`.
    #[arg(short = 'C', long = "directory", value_name = "DIR", global = true)]
    pub directory: Option<PathBuf>,

    /// Path to `cart.toml`. Mirrors cargo's `--manifest-path`. The file
    /// must be named `cart.toml`. A relative path is resolved against
    /// `-C`/`--directory` if given, otherwise against the current dir.
    #[arg(long = "manifest-path", value_name = "PATH", global = true)]
    pub manifest_path: Option<PathBuf>,

    /// Increase log verbosity: default = info, `-v` = debug, `-vv` = trace.
    /// `RUST_LOG` overrides both if set.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,

    #[command(subcommand)]
    pub command: Subcommands,
}

#[derive(Subcommand)]
pub enum Subcommands {
    /// Create a new pack in a new directory (interactive setup).
    New(New),
    /// Initialize an existing directory as a pack (interactive setup).
    Init(Init),
    Build(Build),
    Run(Run),
    List(List),
    Remove(Remove),
    Disable(Disable),
    Enable(Enable),
    Update(Update),
    /// Add a Modrinth mod: interactive picker.
    #[command(alias = "mr")]
    Modrinth(Modrinth),
    /// Add a CurseForge mod: interactive picker.
    #[command(alias = "cf")]
    Curseforge(Curseforge),
    /// Package the pack for redistribution: mrpack, curseforge, prism.
    Export(Export),
}

#[derive(Args)]
pub struct Init {
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(Args)]
pub struct New {
    pub path: PathBuf,
}

#[derive(Args)]
pub struct Build;

#[derive(Args)]
pub struct Run {
    #[clap(flatten)]
    pub build: Build,
}

#[derive(Args)]
pub struct List;

#[derive(Args)]
pub struct Remove {
    /// Name of the mod as it appears under `[mods]` in `cart.toml`.
    pub name: String,
}

#[derive(Args)]
pub struct Enable {
    /// Name of the mod as it appears under `[mods]` in `cart.toml`.
    pub name: String,
}

#[derive(Args)]
pub struct Disable {
    /// Name of the mod as it appears under `[mods]` in `cart.toml`.
    pub name: String,
}

#[derive(Args)]
pub struct Update {
    /// Update only these mods. If empty, updates every Modrinth or
    /// CurseForge entry in the manifest. URL entries are always skipped.
    pub names: Vec<String>,
}

#[derive(Args)]
pub struct Modrinth {
    #[command(subcommand)]
    pub command: ModrinthCommand,
}

#[derive(Subcommand)]
pub enum ModrinthCommand {
    Add(modrinth::Add),
}

pub mod modrinth {
    use clap::Args;

    #[derive(Args)]
    pub struct Add {
        /// Optional starting query. Seeds the picker's input so results
        /// appear immediately; omit to open the picker empty and type
        /// from scratch. Passed to Modrinth's `/v2/search`; you then
        /// pick from the results to add to `[mods]`.
        pub query: Option<String>,

        /// Maximum number of results to offer in the picker.
        #[arg(long, default_value_t = 10)]
        pub limit: u32,

        /// Add the chosen mod already disabled (placed as
        /// `<name>.jar.disabled`).
        #[arg(long)]
        pub disabled: bool,

        /// Skip the dependency-confirmation prompt for the chosen mod.
        #[arg(short = 'y', long)]
        pub yes: bool,
    }
}

#[derive(Args)]
pub struct Curseforge {
    #[command(subcommand)]
    pub command: CurseforgeCommand,
}

#[derive(Subcommand)]
pub enum CurseforgeCommand {
    Add(curseforge::Add),
}

pub mod curseforge {
    use clap::Args;

    #[derive(Args)]
    pub struct Add {
        /// Optional starting query. Seeds the picker's input so results
        /// appear immediately; omit to open the picker empty and type
        /// from scratch. Passed to CurseForge's `/v1/mods/search`; you
        /// then pick from the results to add to `[mods]`.
        pub query: Option<String>,

        /// Maximum number of results to offer in the picker.
        #[arg(long, default_value_t = 10)]
        pub limit: u32,

        /// Add the chosen mod already disabled (placed as
        /// `<name>.jar.disabled`).
        #[arg(long)]
        pub disabled: bool,

        /// Skip the dependency-confirmation prompt for the chosen mod.
        #[arg(short = 'y', long)]
        pub yes: bool,
    }
}

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
