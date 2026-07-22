# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is cart

`cart` is a Minecraft launcher and mod manager CLI written in Rust. It reads a `cart.toml` manifest file (analogous to `Cargo.toml`) that declares the Minecraft version, optional Forge version, and mod dependencies, then downloads mods, manages a local game directory, and launches the game with the correct Java runtime.

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
cargo run -- init <path>              # create a new cart project at <path>
cargo run -- modrinth add <slug>      # add a Modrinth mod (--version to pin, --disabled); alias: `mr add`
cargo run -- modrinth search <query>  # list top Modrinth mod matches (--limit N); alias: `mr search`
cargo run -- modrinth find <query>    # interactive: search, pick from menu, add to manifest; alias: `mr find`
cargo run -- curseforge add <slug>    # add a CurseForge mod (needs CURSEFORGE_API_KEY); alias: `cf add`
cargo run -- remove <name>            # remove a mod from [mods]
cargo run -- enable <name>            # flip the `disabled` flag off
cargo run -- disable <name>           # flip the `disabled` flag on
cargo run -- update [names...]        # re-resolve Modrinth entries; rewrite pinned version
cargo run -- list                     # print [mods] as a table
cargo run -- build                    # download mods into minecraft/mods/, mirror src/
cargo run -- run                      # build + launch Minecraft
cargo run -- -C <dir> run             # explicit project directory (contains cart.toml)
cargo run -- --mv 1.20.4 run          # override Minecraft version
```

## Architecture

The crate is structured as both a binary and a library. `src/main.rs` is the binary entry point; `src/lib.rs` exposes the public API (`Launcher`, `Instance`, `ModCache`, `Sha1Digest`).

**Data flow for `run`:**
1. `Config::load` resolves the manifest — walks up from `cwd` looking for `cart.toml`, or uses the `-C`/`--directory` flag to target a specific project dir
2. `Build::run_with` resolves each `[mods]` entry to a concrete URL (Modrinth entries hit `api.modrinth.com` to pick a compatible version; URL entries pass through), fetches jars via `ModCache`, prunes stale top-level jars from `minecraft/mods/`, and hard-links the current set in
3. `Build::run_with` also mirrors `<manifest_dir>/src/` on top of `<manifest_dir>/minecraft/` — pack-authored configs/resources overwrite whatever's there. Top-level jars in `src/mods/` are rejected loudly
4. `Launcher` (via `Cache`) fetches version manifests, Java distribution, game client jar, and library jars from Mojang's Piston API, storing them content-addressed on disk. If `forge` is set on the manifest, the Forge installer is fetched and its libraries are added to the classpath
5. The game is launched with the bundled Java binary, using a temp dir for natives

**Key types:**
- `Manifest` (`src/manifest.rs`) — `cart.toml` deserialization. Fields: `minecraft: String`, `loader: Option<cart::Loader>`, `mods: HashMap<String, ModDependency>`
- `Loader` (`src/launcher/loader.rs`) — `{ kind: LoaderKind, spec: LoaderSpec }` where `LoaderKind ∈ {Forge, Fabric, NeoForge}` and `LoaderSpec ∈ {Latest, Recommended, Pinned(String)}`. `Recommended` is Forge-only, rejected at parse time for Fabric and NeoForge.
- `ModDependency` (`src/manifest/mod_dependency.rs`) — untagged serde enum with two variants: `Modrinth { modrinth, version, disabled }` (loose if `version` is `None`) and `Url { url, disabled }`. `filename()` returns `<name>.jar` or `<name>.jar.disabled`
- `manifest::document` — `toml_edit`-based helpers (`load_document`, `save_document`, `add_modrinth_mod`, `remove_mod`, `set_mod_disabled`, `set_mod_version`). All the mutating subcommands go through these so comments and formatting in `cart.toml` are preserved
- `Config` (`src/config.rs`) — merges manifest with CLI flag overrides (e.g. `--mv` overrides `minecraft` field)
- `Cache` (`src/launcher/cache.rs`) — maps URLs to local paths under the cache dir (`~/.cache/cart/`); SHA-1 verified on download
- `AssetCache` — manages Minecraft asset objects (sounds, textures) under the cache dir; reads the asset index JSON
- `ModCache` — thin wrapper over `Cache` for mod jars
- `Instance` / `InstanceBuilder` — holds the game directory, Minecraft version, and optional Forge spec passed to `Launcher::launch`
- `Launcher` / `LauncherBuilder` — orchestrates the full launch sequence; `LauncherBuilder::cache_dir` lets callers override the cache path (used in tests)

**External APIs:**
- `src/api/piston/` — serde types for all Mojang API responses: version manifest, per-version manifest, Java distribution manifest, asset index, library rules, and native classifiers. Platform detection for native libraries lives in `NativeClassifier::current()` and `OsName::matches_current_platform()`.
- `src/api/modrinth.rs` — `resolve()` picks a Modrinth version by slug given a `minecraft_version` + `loader`. Called from `cart add`, `cart update`, and `cart build`. Loader string comes from `manifest.loader.kind`: `"forge"`, `"fabric"`, or `"vanilla"` when no loader is set.
- `src/api/forge.rs` — Forge installer/version metadata.

**Note on duplication:** the `cli/run.rs` command composes `Build::run_with` directly (sharing a `Launcher`) rather than shelling out to `Build::run`, so build+launch reuse one config load.

## Manifest format (`cart.toml`)

```toml
minecraft = "1.20.1"
# loader is optional. Bare string → latest of that loader.
# loader = "fabric"                                        # → Fabric latest
# loader = "forge"                                         # → Forge latest
# loader = { forge = "recommended" }                       # Forge stable channel
# loader = { forge = "47.3.12" }                           # pinned Forge build
# loader = { fabric = "0.15.7" }                           # pinned Fabric loader
loader = { forge = "47.2.0" }

[mods]
jei = { modrinth = "jei", version = "15.2.0.27" }          # Modrinth, pinned
appleskin = { modrinth = "appleskin" }                     # Modrinth, loose
custom = { url = "https://example.com/CustomMod.jar" }     # raw URL
wip = { modrinth = "some-mod", disabled = true }           # placed as wip.jar.disabled
```

The game directory is always `<manifest_dir>/minecraft/`; mods are placed in `<manifest_dir>/minecraft/mods/`. A sibling `<manifest_dir>/src/` directory, if present, is copied on top of `minecraft/` on every build — top-level jars under `src/mods/` are rejected (mods must be declared in `[mods]`).
