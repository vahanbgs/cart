//! Emit shell completions for cart's CLI into `$OUT_DIR` at build time.
//!
//! `flake.nix` picks these up post-install via `installShellCompletion`,
//! reading from `target/*/build/cart-*/out/{cart.bash,_cart,cart.fish,cart.nu}`
//! (Cargo's build-script output convention).
//!
//! ## Single source of truth
//!
//! The clap-derive types live in `src/cli/args.rs`. That file has zero
//! `crate::*` imports, which lets us include it from here via
//! `#[path = "..."]` before the crate itself compiles. Both `main.rs`
//! (through `mod cli;`) and this build script share the exact same
//! derive — no drift.

use std::{env, path::PathBuf};

use clap::CommandFactory;
use clap_complete::{Shell, generate_to};
use clap_complete_nushell::Nushell;

#[path = "src/cli/args.rs"]
mod args;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/cli/args.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let mut cmd = args::Cli::command();

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        generate_to(shell, &mut cmd, "cart", &out_dir)
            .unwrap_or_else(|e| panic!("generate {shell} completion: {e}"));
    }
    generate_to(Nushell, &mut cmd, "cart", &out_dir).expect("generate nushell completion");
}
