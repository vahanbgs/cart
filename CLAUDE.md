# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is cart

`cart` is a Minecraft launcher and mod manager CLI written in Rust. It reads a `cart.toml` manifest file (analogous to `Cargo.toml`) that declares the Minecraft version and mod dependencies, then downloads mods, manages a local game directory, and launches the game with the correct Java runtime.

## Commands

```sh
# Enter dev shell (or use direnv)
nix develop

cargo build              # debug build
cargo build --release    # release build
cargo clippy             # lint
cargo fmt                # format
cargo test               # run tests

# Run the CLI
cargo run -- init <path>          # create a new cart project at <path>
cargo run -- build                # download mods into minecraft/mods/
cargo run -- run                  # build + launch Minecraft
cargo run -- --manifest <path> run  # explicit manifest path
cargo run -- --mv 1.20.4 run      # override Minecraft version
```

## Architecture

The crate is structured as both a binary and a library. `src/main.rs` is the binary entry point; `src/lib.rs` exposes the public API (`Launcher`, `Instance`, `Sha1Digest`).

**Data flow for `run`:**
1. `Config::load` resolves the manifest — walks up from `cwd` looking for `cart.toml`, or uses `--manifest` flag
2. `Launcher` (via `Cache`) fetches version manifests, Java distribution, game client jar, and library jars from Mojang's Piston API, storing them content-addressed on disk
3. Mods are fetched from their URLs via `ModCache` and hard-linked into `<manifest_dir>/minecraft/mods/`
4. The game is launched with the bundled Java binary, using a temp dir for natives

**Key types:**
- `Manifest` (`src/manifest.rs`) — `cart.toml` deserialization; `ModDependency` is currently only `{ url }` but is an untagged enum ready for more variants
- `Config` (`src/config.rs`) — merges manifest with CLI flag overrides (e.g. `--mv` overrides `minecraft` field)
- `Cache` (`src/launcher/cache.rs`) — maps URLs to local paths under the cache dir (`~/.cache/cart/`); SHA-1 verified on download
- `AssetCache` — manages Minecraft asset objects (sounds, textures) under the cache dir; reads the asset index JSON
- `ModCache` — thin wrapper over `Cache` for mod jars
- `Instance` / `InstanceBuilder` — holds the game directory and version string passed to `Launcher::launch`
- `Launcher` / `LauncherBuilder` — orchestrates the full launch sequence; `LauncherBuilder::cache_dir` lets callers override the cache path (used in tests)

**`src/api/piston/`** contains serde types for all Mojang API responses: version manifest, per-version manifest, Java distribution manifest, asset index, library rules, and native classifiers. Platform detection for native libraries lives in `NativeClassifier::current()` and `OsName::matches_current_platform()`.

**Note on duplication:** `cli/run.rs` currently duplicates the mod-download logic from `cli/build.rs` rather than calling `Build::run`. This is intentional until the build output path is more formally defined.

## Manifest format (`cart.toml`)

```toml
minecraft = "1.12.2"

[mods]
hei = { url = "https://example.com/HadEnoughItems.jar" }
```

The game directory is always `<manifest_dir>/minecraft/`; mods are placed in `<manifest_dir>/minecraft/mods/`.
