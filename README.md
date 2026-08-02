# cart

> A Minecraft modpack manager that behaves like Cargo.

Declare your pack in one file. Commit it. Run it. Anywhere.

```toml
# cart.toml
minecraft = "1.20.1"
loader = "forge"

[mods]
jei = { modrinth = "jei" }
appleskin = { modrinth = "appleskin" }
```

```sh
$ cart run
```

cart fetches the right Java runtime, the Mojang client, Forge, and every
mod — content-addressed and SHA-1 verified — then launches the game.
Second time around, everything's cached.

## Why you'd want this

- **Your pack is a text file.** Diff it. Commit it. `git clone`, `cart run`, play.
- **Loose *or* pinned.** `modrinth = "jei"` picks the newest compatible build; add `version = "…"` when you need reproducibility. `cart update` bumps every loose entry at once.
- **Modrinth + CurseForge + raw URLs.** All in the same manifest, side by side.
- **Interactive discovery.** `cart mr add create` searches Modrinth, shows a menu, adds your pick to the manifest.
- **One cache, many packs.** Try three 1.20.1 modpacks — the JEI jar sits on disk once. Everything under `~/.cache/cart/` is content-addressed and SHA-1 verified.
- **No Java on your machine? Fine.** cart pulls the Mojang-shipped JRE for whatever Minecraft version you're launching.
- **Forge, Fabric, NeoForge.** Bare string for "latest," a table for a pinned build, or `{ forge = "recommended" }` for the Forge stable channel.

## Install

`cart` is a Rust crate with a pinned Nix dev shell:

```sh
nix develop            # or: direnv allow
cargo build --release
```

The binary lands at `target/release/cart`. Drop it on your `PATH`.

## Sixty seconds from empty to launched

```sh
cart new mypack                # interactive: pick MC version + loader
cd mypack
cart mr add jei                # search Modrinth, pick from a menu
cart run                       # download + launch
```

`cart new <path>` scaffolds a fresh directory; `cart init [path]`
scaffolds inside an existing one (default `.`). `cart mr add` (Modrinth)
and `cart cf add` (CurseForge) each open an interactive picker to search
and add mods to the manifest. CurseForge needs a `CURSEFORGE_API_KEY` in
the environment.

## The manifest

```toml
minecraft = "1.20.1"

# Loader is optional. Bare string → latest of that loader.
# loader = "fabric"
# loader = "forge"
# loader = "neoforge"
#
# Or pin a specific build:
# loader = { forge = "47.2.0" }
# loader = { forge = "recommended" }        # Forge stable channel
# loader = { fabric = "0.15.7" }
loader = { forge = "47.2.0" }

[mods]
# Modrinth, pinned to a version_number:
jei = { modrinth = "jei", version = "15.2.0.27" }

# Modrinth, loose — newest compatible build at build time:
appleskin = { modrinth = "appleskin" }

# CurseForge — always pinned to (project id, file id).
# `cart cf add` resolves slugs and writes the ids for you; CF slugs
# can rename, ids are permanent.
create = { curseforge = 328085, file = 6116881 }

# Raw URL — fully user-pinned; `cart update` skips these:
custom = { url = "https://example.com/CustomMod.jar" }

# Any entry can be disabled — cart writes it as `<name>.jar.disabled`:
wip = { modrinth = "some-mod", disabled = true }
```

Alongside `cart.toml` you can keep an `overrides/` tree that mirrors
the game directory layout. Every `cart build` copies it on top of
`minecraft/`, so pack-authored configs, resource packs, and scripts
travel with the manifest. Top-level jars in `overrides/mods/` are
rejected — mods belong in `[mods]`.

## Commands

| Command | What it does |
| --- | --- |
| `cart new <path>` | Interactive setup in a fresh directory. Errors if `<path>` exists. |
| `cart init [path]` | Same, but for an existing directory (default `.`). Errors if `cart.toml` is already there. |
| `cart mr add [query]` | Interactive: search Modrinth, pick from a menu, add to the manifest. `--disabled` adds it disabled. |
| `cart cf add [query]` | Same, against CurseForge. Requires `CURSEFORGE_API_KEY`. |
| `cart remove <name>` | Remove a mod entry from `[mods]`. |
| `cart enable <name>` / `cart disable <name>` | Flip the `disabled` flag. |
| `cart update [names...]` | Re-resolve Modrinth + CurseForge entries against the current Minecraft version. URLs are skipped. |
| `cart list` | Print `[mods]` as a table. |
| `cart build` | Download mods into `minecraft/mods/` and mirror `overrides/` on top of `minecraft/`. |
| `cart run` | `build`, then launch Minecraft with the bundled Java runtime. |
| `cart export <format>` | Package the pack as `mrpack`, `curseforge`, or `prism` for redistribution. |

Global flags:

- `-C, --directory <DIR>` — treat `<DIR>` as the project root (mirrors
  `cargo -C` / `make -C`). Default: walk up from cwd looking for `cart.toml`.
- `--mv <version>` — override the manifest's Minecraft version for this run.
- `-v, -vv` — bump log verbosity (default `info`). `RUST_LOG` overrides both.

## Layout on disk

```
mypack/
├── cart.toml                  # manifest
├── overrides/                 # mirrored into minecraft/ on every build
│   └── config/…
└── minecraft/                 # game directory (created by cart)
    ├── mods/                  # cart-managed; jars are hardlinks into the cache
    ├── saves/
    └── …
```

The cache at `~/.cache/cart/` holds Minecraft version manifests, the
Java runtime, library and native jars, asset objects, and mod jars —
all content-addressed, SHA-1 verified, and shared across every pack
on the machine.
