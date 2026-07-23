# cart

A Minecraft launcher and mod manager for the command line. `cart` reads a
`cart.toml` manifest — the analogue of `Cargo.toml` — that declares the
Minecraft version, an optional mod loader (Forge, Fabric, or NeoForge),
and a list of mods. It downloads everything into a local, per-project
game directory and launches the game with the right Java runtime.

Mods can be pulled from Modrinth by slug, from CurseForge by numeric
project/file id, or from any raw URL. Downloads are content-addressed
under `~/.cache/cart/` and SHA-1 verified, so switching between projects
and Minecraft versions re-uses the cache.

## Install

`cart` is a Rust crate. A `flake.nix` is provided for a pinned dev shell.

```sh
nix develop           # or use direnv
cargo build --release
```

The binary lands at `target/release/cart`.

## Quick start

```sh
cart init mypack               # interactive: pick MC version + loader
cd mypack
cart mr find jei               # interactive: search Modrinth, pick, add
cart run                       # download mods, then launch Minecraft
```

`cart init` prompts for the Minecraft version and mod loader; pass `--mv
<version>` beforehand to skip the version prompt. `cart mr` (Modrinth)
and `cart cf` (CurseForge) each expose `add`, `search`, and `find` — `add`
takes a slug when you already know the exact mod, `search` prints hits to
stdout, and `find` is the interactive picker. CurseForge subcommands
require a `CURSEFORGE_API_KEY` in the environment.

## Manifest format

```toml
minecraft = "1.20.1"

# Loader is optional. A bare string picks the latest build of that loader:
# loader = "fabric"
# loader = "forge"
# loader = "neoforge"
# Or pin a specific build:
# loader = { forge = "recommended" }        # Forge stable channel
# loader = { fabric = "0.15.7" }            # pinned Fabric loader
loader = { forge = "47.2.0" }

[mods]
# Modrinth, pinned to a specific version_number:
jei = { modrinth = "jei", version = "15.2.0.27" }

# Modrinth, loose — resolved to the newest compatible version at build time:
appleskin = { modrinth = "appleskin" }

# CurseForge — always pinned to a (project id, file id) pair.
# `cart cf add <slug>` and `cart cf find` resolve slugs and write these
# ids for you; CurseForge slugs can rename, ids are permanent.
create = { curseforge = 328085, file = 6116881 }

# Raw URL — fully user-pinned; cart update skips these:
custom = { url = "https://example.com/CustomMod.jar" }

# Any entry can be disabled — placed on disk as `<name>.jar.disabled`:
wip = { modrinth = "some-mod", disabled = true }
```

The manifest directory also owns a `src/` tree that mirrors the
`.minecraft/` layout. Every `cart build` replicates `src/` on top of
`minecraft/`, so pack-authored configs, resource packs, or scripts live
in version control alongside the manifest. Top-level jars in `src/mods/`
are rejected — mods belong in `[mods]`.

## Commands

| Command | What it does |
| --- | --- |
| `cart init <path>` | Interactive project setup — prompts for Minecraft version and mod loader, writes `cart.toml`. |
| `cart mr add <slug>` | Add a Modrinth mod. `--version` pins; `--name` overrides the manifest key; `--disabled` adds it disabled. |
| `cart mr search <query>` | List top Modrinth matches for `<query>`. |
| `cart mr find <query>` | Interactive: search Modrinth, pick from the menu, add to the manifest. |
| `cart cf add <slug>` / `cf search <query>` / `cf find <query>` | The same three verbs against CurseForge. Requires `CURSEFORGE_API_KEY`. |
| `cart remove <name>` | Remove a mod entry from `[mods]`. |
| `cart enable <name>` / `cart disable <name>` | Flip the `disabled` flag on an entry. |
| `cart update [names...]` | Re-resolve Modrinth and CurseForge entries against the current Minecraft version and rewrite the pinned version/file. URL entries are skipped. |
| `cart list` | Print `[mods]` as a table. |
| `cart build` | Download mods into `minecraft/mods/` and copy `src/` over `minecraft/`. |
| `cart run` | `build`, then launch Minecraft with the bundled Java runtime. |
| `cart export <format>` | Package the pack as `mrpack`, `curseforge`, or `prism` for redistribution. |

Global flags:

- `-C, --directory <DIR>` — treat `<DIR>` as the project root (mirrors
  `cargo -C` / `make -C`). Default: walk up from cwd looking for `cart.toml`.
- `--mv <version>` — override the manifest's Minecraft version for this run.
- `-v, -vv` — bump log verbosity (default `info`). `RUST_LOG` overrides both.

## Layout

```
mypack/
├── cart.toml                  # manifest
├── src/                       # mirrored into minecraft/ on every build
│   └── config/…
└── minecraft/                 # game directory (created by cart)
    ├── mods/                  # cart-managed; jars are hardlinks into the cache
    ├── saves/
    └── …
```

The cache lives at `~/.cache/cart/` and holds Minecraft version manifests,
the Java runtime, library and native jars, asset objects, and mod jars —
all content-addressed and SHA-1 verified.
