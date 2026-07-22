//! Emit shell completions for cart's CLI into `$OUT_DIR` at build time.
//!
//! `flake.nix` picks these up post-install via `installShellCompletion`,
//! reading from `target/*/build/cart-*/out/{cart.bash,_cart,cart.fish,cart.nu}`
//! (Cargo's build-script output convention).
//!
//! ## Why the CLI is redeclared here
//!
//! The main crate uses derive-based clap, but `build.rs` runs *before*
//! the crate compiles — it can't reach `crate::cli::Cli`. Restructuring
//! ten submodules just to share the derive is a bigger change than the
//! completions feature warrants, so this script mirrors the CLI shape
//! via clap's builder API instead. If you add or rename a subcommand /
//! argument in `src/cli/*.rs`, update it here too — the
//! `cargo:rerun-if-changed=src/cli.rs` hint below at least makes edits
//! trigger a rebuild, so drift shows up on the next `cargo build`.

use std::{env, path::PathBuf};

use clap::{Arg, ArgAction, Command};
use clap_complete::{Shell, generate_to};
use clap_complete_nushell::Nushell;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/cli.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let mut cmd = cart_cli();

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        generate_to(shell, &mut cmd, "cart", &out_dir)
            .unwrap_or_else(|e| panic!("generate {shell} completion: {e}"));
    }
    generate_to(Nushell, &mut cmd, "cart", &out_dir)
        .expect("generate nushell completion");
}

fn cart_cli() -> Command {
    // Args shared by the modrinth/curseforge subcommand trees. `add`,
    // `search`, and `find` take the same shape on both source modules.
    let add_args = |cmd: Command| {
        cmd.arg(Arg::new("slug").required(true))
            .arg(Arg::new("version").long("version").value_name("VERSION"))
            .arg(Arg::new("name").long("name").value_name("NAME"))
            .arg(
                Arg::new("disabled")
                    .long("disabled")
                    .action(ArgAction::SetTrue),
            )
    };
    let query_and_limit = |cmd: Command| {
        cmd.arg(Arg::new("query").required(true))
            .arg(Arg::new("limit").long("limit").value_name("N"))
    };
    let source_tree = |name: &'static str, alias: &'static str| {
        Command::new(name)
            .alias(alias)
            .subcommand_required(true)
            .subcommand(add_args(Command::new("add")))
            .subcommand(query_and_limit(Command::new("search")))
            .subcommand(query_and_limit(Command::new("find")))
    };

    Command::new("cart")
        .arg(
            Arg::new("directory")
                .short('C')
                .long("directory")
                .value_name("DIR")
                .global(true),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::Count)
                .global(true),
        )
        .arg(
            Arg::new("minecraft_version")
                .long("mv")
                .value_name("VERSION"),
        )
        .subcommand_required(true)
        .subcommand(Command::new("init").arg(Arg::new("path").required(true)))
        .subcommand(Command::new("build"))
        .subcommand(Command::new("run"))
        .subcommand(Command::new("list"))
        .subcommand(Command::new("remove").arg(Arg::new("name").required(true)))
        .subcommand(Command::new("enable").arg(Arg::new("name").required(true)))
        .subcommand(Command::new("disable").arg(Arg::new("name").required(true)))
        .subcommand(Command::new("update").arg(Arg::new("names").num_args(0..)))
        .subcommand(source_tree("modrinth", "mr"))
        .subcommand(source_tree("curseforge", "cf"))
        .subcommand(
            Command::new("export")
                .arg(
                    Arg::new("format")
                        .required(true)
                        .value_parser(["mrpack", "curseforge", "prism"]),
                )
                .arg(
                    Arg::new("output")
                        .short('o')
                        .long("output")
                        .value_name("PATH"),
                ),
        )
}
