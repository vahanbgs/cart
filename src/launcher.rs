mod arguments;
mod cache;
pub mod fabric;
pub mod forge;
mod fs_ops;
mod instance;
mod java;
mod loader;

pub use instance::Instance;
pub use loader::{Loader, LoaderKind, LoaderSpec};
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
};

use std::{collections::HashMap, path::PathBuf, process::Stdio};

use anyhow::anyhow;
use directories_next::ProjectDirs;
use reqwest::Client;
use url::Url;

use crate::api::{
    Endpoint,
    forge::ForgePromotions,
    piston::{Arguments, Version, VersionManifest},
};
use cache::{AssetCache, Cache};

pub struct Launcher {
    cache: Cache,
    /// Shared HTTP client for one-shot uncached GETs (Fabric loader listing,
    /// NeoForge maven metadata). `reqwest::Client` is `Arc`-internal, so
    /// cloning it into `Cache` and out to those callers shares the same
    /// connection pool.
    client: Client,
}

impl Default for Launcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Launcher {
    pub fn new() -> Self {
        let project_dirs =
            ProjectDirs::from("", "", "cart").expect("Could not find valid home directory path");
        let cache_dir = project_dirs.cache_dir();
        let client = Client::new();
        let cache = Cache::new(cache_dir.to_owned(), client.clone());

        Self { cache, client }
    }

    pub fn builder() -> LauncherBuilder {
        Default::default()
    }

    /// Assemble the full Java command that would launch `instance`,
    /// including asset/Java/mod resolution and Forge install where
    /// requested, plus the temporary natives directory it references.
    /// Split out from `launch` so tests can spawn on their own timeline
    /// — the `TempDir` must outlive the child process.
    pub async fn build_command(&self, instance: &Instance) -> anyhow::Result<(Command, TempDir)> {
        let version_manifest = self
            .cache
            .fetch_json::<VersionManifest>(VersionManifest::url(), None)
            .await?;

        let version_map = version_manifest.version_map();

        let version_id = if instance.version() == "latest" {
            version_manifest.latest_release()
        } else {
            instance.version()
        };

        let version_info = version_map.get(version_id).expect("unknown version");
        let version = self
            .cache
            .fetch_json::<Version>(&version_info.url, Some(&version_info.sha1))
            .await?;

        let asset_index = &version.asset_index;
        let asset_cache = AssetCache::new(&self.cache);
        asset_cache.update(asset_index).await?;
        let asset_directory = asset_cache.directory();

        let java_path =
            java::fetch_java_distribution(version.java_version.component, &self.cache).await?;

        // Fetch the vanilla client JAR (needed both for vanilla launch and as
        // input to the Forge processor pipeline).
        let vanilla_client_jar = self
            .cache
            .fetch(
                &version.downloads.client.url,
                Some(&version.downloads.client.sha1),
            )
            .await?;

        let natives_directory = tempfile::tempdir()?;

        // ── Loader ────────────────────────────────────────────────────────────
        let mut resolved_forge_family: Option<(forge::ForgeFlavor, String)> = None;
        let (client_jar, forge_extra_libraries, forge_main_class, forge_game_args, forge_jvm_args) =
            match instance.loader().map(|l| (l, l.kind)) {
                None => (vanilla_client_jar, vec![], None, (None, vec![]), vec![]),
                Some((loader, LoaderKind::NeoForge)) => {
                    let neoforge_version = match &loader.spec {
                        LoaderSpec::Recommended => {
                            anyhow::bail!(
                                "NeoForge has no `recommended` channel; use `latest` or a pinned version"
                            );
                        }
                        LoaderSpec::Pinned(v) => v.clone(),
                        LoaderSpec::Latest => {
                            let metadata =
                                crate::api::neoforge::MavenMetadata::fetch(&self.client).await?;
                            metadata.latest_stable_for_mc(version_id).ok_or_else(|| {
                                anyhow!("no stable NeoForge release for Minecraft {version_id}")
                            })?
                        }
                    };
                    tracing::info!("resolved NeoForge for mc={version_id} to {neoforge_version}");

                    let result = forge::install(
                        forge::ForgeFlavor::NeoForge,
                        &neoforge_version,
                        &vanilla_client_jar,
                        &java_path,
                        &self.cache,
                    )
                    .await?;

                    let fv = result.version;
                    let game_args = fv.minecraft_arguments.clone();
                    let jvm_args = fv.arguments.jvm.clone();
                    let game_args_modern = fv.arguments.game.clone();

                    // NeoForge's `-DignoreList` (from its version.json) filters
                    // `client-extra,${version_name}.jar` from modlauncher's
                    // module scan — but NOT `neoforge-`. Two consequences:
                    //
                    //   1. Putting the PATCHED `neoforge-<ver>-client.jar` on
                    //      the classpath would let Java auto-modularize it as
                    //      module `neoforge`, colliding with the `FML-System-
                    //      Mods: neoforge` module discovered in `library_
                    //      directory` via `-universal.jar`. So we use the
                    //      vanilla client jar instead. PATCHED still lives in
                    //      `library_directory` where FMLLoader finds it.
                    //
                    //   2. The vanilla client jar's cache filename is
                    //      `client.jar` (from its piston-data URL). Java
                    //      would auto-modularize it as module `client`,
                    //      exporting `com.mojang.blaze3d.platform` — colliding
                    //      with the `minecraft` module FML derives from
                    //      PATCHED. Match `${version_name}.jar` in the
                    //      ignoreList by hardlinking it under
                    //      `<cache>/versions/<vid>/<vid>.jar`, mirroring
                    //      what vanilla launchers do.
                    let versioned_client = self
                        .cache
                        .versions_dir()
                        .join(&version.id)
                        .join(format!("{}.jar", &version.id));
                    // materialize is a cache-hit here (we already fetched
                    // the client jar into `vanilla_client_jar`); it just
                    // hard-links the cached path into the versioned alias.
                    self.cache
                        .materialize(
                            &version.downloads.client.url,
                            Some(&version.downloads.client.sha1),
                            &versioned_client,
                        )
                        .await?;
                    let client_jar = versioned_client;

                    resolved_forge_family =
                        Some((forge::ForgeFlavor::NeoForge, result.effective_version));

                    (
                        client_jar,
                        fv.libraries,
                        Some(fv.main_class),
                        (game_args, game_args_modern),
                        jvm_args,
                    )
                }
                Some((loader, LoaderKind::Forge)) => {
                    let forge_channel = match &loader.spec {
                        LoaderSpec::Latest => "latest",
                        LoaderSpec::Recommended => "recommended",
                        LoaderSpec::Pinned(v) => v.as_str(),
                    };
                    let forge_version =
                        if matches!(&loader.spec, LoaderSpec::Latest | LoaderSpec::Recommended) {
                            let promotions = self
                                .cache
                                .fetch_json::<ForgePromotions>(ForgePromotions::url(), None)
                                .await?;
                            promotions.resolve(version_id, forge_channel).ok_or_else(|| {
                            anyhow!("no Forge {forge_channel} release for Minecraft {version_id}")
                        })?
                        } else {
                            format!("{version_id}-{forge_channel}")
                        };

                    let result = forge::install(
                        forge::ForgeFlavor::Forge,
                        &forge_version,
                        &vanilla_client_jar,
                        &java_path,
                        &self.cache,
                    )
                    .await?;

                    let fv = result.version;
                    let game_args = fv.minecraft_arguments.clone();
                    let jvm_args = fv.arguments.jvm.clone();
                    let game_args_modern = fv.arguments.game.clone();

                    let client_jar = result
                        .patched_client_jar
                        .unwrap_or_else(|| vanilla_client_jar.clone());

                    resolved_forge_family =
                        Some((forge::ForgeFlavor::Forge, result.effective_version));

                    (
                        client_jar,
                        fv.libraries,
                        Some(fv.main_class),
                        (game_args, game_args_modern),
                        jvm_args,
                    )
                }
                Some((loader, LoaderKind::Fabric)) => {
                    // Fabric: no client-JAR patching, no legacy game-args
                    // string. Extra libs come from the profile JSON, the
                    // JVM boots KnotClient, and the modern game/jvm args
                    // are merged on top of vanilla's.
                    let result =
                        fabric::install(version_id, &loader.spec, &self.cache, &self.client)
                            .await?;
                    (
                        vanilla_client_jar,
                        result.libraries,
                        Some(result.main_class),
                        (None, result.game_args),
                        result.jvm_args,
                    )
                }
            };

        // Forge-family extras have empty artifact URLs; their cache paths
        // are derived from the flavor's maven base. Vanilla and Fabric
        // pass None (Fabric always populates artifact.url directly).
        let forge_family_maven_base = resolved_forge_family
            .as_ref()
            .map(|(f, _)| f.maven_base_url().clone());
        let classpath = java::build_class_path(
            &version,
            &client_jar,
            &forge_extra_libraries,
            forge_family_maven_base.as_ref(),
            &natives_directory,
            &self.cache,
        )
        .await?;

        fs::create_dir_all(instance.directory()).await?;

        let variables: HashMap<&str, String> = [
            ("auth_player_name", "OfflinePlayer".to_owned()),
            ("version_name", version.id.clone()),
            (
                "game_directory",
                instance.directory().to_string_lossy().into_owned(),
            ),
            (
                "assets_root",
                asset_directory.to_string_lossy().into_owned(),
            ),
            ("assets_index_name", asset_index.id.clone()),
            // Pre-1.6 versions use the combined `${auth_session}` token
            // (`token:<accessToken>:<uuid>`) instead of the split
            // `--accessToken`/`--uuid` args. Offline launchers use a
            // zero-padded stub — MC only logs it and uses it for network
            // calls that offline auth skips anyway.
            (
                "auth_session",
                "token:0:00000000-0000-0000-0000-000000000000".to_owned(),
            ),
            // Pre-1.6's `--assetsDir ${game_assets}` points at a
            // name-based (not hash-based) asset directory. We advertise
            // the well-known `assets/virtual/<index>` path here so tier-A
            // stops seeing an unresolved template; actually materializing
            // the assets in that directory is the tier-B follow-up.
            (
                "game_assets",
                asset_directory
                    .join("virtual")
                    .join(&asset_index.id)
                    .to_string_lossy()
                    .into_owned(),
            ),
            (
                "auth_uuid",
                "00000000-0000-0000-0000-000000000000".to_owned(),
            ),
            ("auth_access_token", "0".to_owned()),
            // Pre-1.13 versions pass `--userProperties ${user_properties}`
            // through to `Main.main`, which parses it as JSON via Gson.
            // An unresolved template makes it crash with
            // "Expected BEGIN_OBJECT but was STRING at line 1 column 1".
            // Modern launchers use `{}` for offline auth.
            ("user_properties", "{}".to_owned()),
            ("user_type", "legacy".to_owned()),
            ("version_type", version.kind.as_ref().to_owned()),
            (
                "natives_directory",
                natives_directory.path().to_string_lossy().into_owned(),
            ),
            ("launcher_name", "cart".to_owned()),
            ("launcher_version", env!("CARGO_PKG_VERSION").to_owned()),
            ("classpath", classpath),
            // Forge 1.17+ uses these to build the JPMS module path (`-p`) and
            // for FMLLoader to locate processor outputs (PATCHED, MC_SRG, …).
            // Must match the local dir used by the Forge install pipeline.
            (
                // Resolves to whichever forge-family flavor is active
                // (or Forge as a harmless default for vanilla/Fabric,
                // which don't reference this template).
                "library_directory",
                forge::local_maven_dir(
                    resolved_forge_family
                        .as_ref()
                        .map(|(f, _)| *f)
                        .unwrap_or(forge::ForgeFlavor::Forge),
                    &self.cache,
                )
                .to_string_lossy()
                .into_owned(),
            ),
            ("classpath_separator", ":".to_owned()),
        ]
        .into_iter()
        .collect();

        let main_class = forge_main_class.as_deref().unwrap_or(&version.main_class);

        let mut command = java::java_binary(&java_path);
        command.current_dir(instance.directory());
        command.args(["-Xmx4G", "-Xms1G"]);

        match &version.arguments {
            Arguments::Modern { jvm, game } => {
                // Vanilla JVM args first, then Forge JVM args.
                command.args(arguments::resolve(jvm, &variables));
                command.args(arguments::resolve(&forge_jvm_args, &variables));
                command.arg(main_class);
                // Vanilla game args first, then Forge game args.
                command.args(arguments::resolve(game, &variables));
                match &forge_game_args {
                    (Some(legacy), _) => {
                        command.args(
                            legacy
                                .split_ascii_whitespace()
                                .map(|t| arguments::substitute(t, &variables)),
                        );
                    }
                    (None, modern) => {
                        command.args(arguments::resolve(modern, &variables));
                    }
                }
            }
            Arguments::Legacy(minecraft_arguments) => {
                command.args([
                    arguments::substitute("-Djava.library.path=${natives_directory}", &variables),
                    "-cp".to_owned(),
                    arguments::substitute("${classpath}", &variables),
                ]);
                command.arg(main_class);
                // Forge's minecraftArguments is a superset of vanilla's (it adds
                // --tweakClass etc.) so use it instead of vanilla when present.
                let game_args = forge_game_args.0.as_deref().unwrap_or(minecraft_arguments);
                command.args(
                    game_args
                        .split_ascii_whitespace()
                        .map(|token| arguments::substitute(token, &variables)),
                );
            }
        }

        match &resolved_forge_family {
            Some((flavor, fv)) => {
                tracing::info!("launching minecraft {} with {flavor:?} {fv}", version.id)
            }
            None => tracing::info!("launching minecraft {}", version.id),
        }

        Ok((command, natives_directory))
    }

    /// Assemble the launch command and wait for Minecraft to exit. A
    /// non-zero exit is logged but not surfaced — Minecraft prints
    /// crashes to `logs/latest.log`, and callers of `run` shouldn't get
    /// an error just because the user closed the game.
    ///
    /// Minecraft's stdout/stderr are piped rather than inherited: each
    /// line is re-emitted as a `tracing::trace!` event under the
    /// `minecraft::stdout` / `minecraft::stderr` targets. The default
    /// filter (see `main.rs`) drops those, so cart's terminal stays
    /// clean. `cart -vv run` opts them back in.
    pub async fn launch(&self, instance: &Instance) -> anyhow::Result<()> {
        let (mut command, _natives_directory) = self.build_command(instance).await?;

        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;

        let stdout_task = child
            .stdout
            .take()
            .map(|s| tokio::spawn(forward_lines(s, "minecraft::stdout")));
        let stderr_task = child
            .stderr
            .take()
            .map(|s| tokio::spawn(forward_lines(s, "minecraft::stderr")));

        let status = child.wait().await?;

        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        if let Some(task) = stderr_task {
            let _ = task.await;
        }

        if !status.success() {
            let log_path = instance.directory().join("logs").join("latest.log");
            tracing::warn!("minecraft exited with {status}; see {}", log_path.display());
        }

        Ok(())
    }

    /// Fetch a mod jar and return its cache path. Thin wrapper over
    /// [`Cache::fetch`] with no expected SHA — mod URLs come from Modrinth
    /// / CurseForge / user configs and don't carry a known digest to
    /// verify against.
    pub async fn fetch_mod(&self, url: &Url) -> anyhow::Result<PathBuf> {
        self.cache.fetch(url, None).await
    }

    /// Whether a mod jar for `url` is already resident in the cache.
    /// Used by the CLI to distinguish "cached" vs "download" in build
    /// progress logging. Racy against a concurrent writer's atomic-rename,
    /// which is fine for logging.
    pub async fn is_mod_cached(&self, url: &Url) -> anyhow::Result<bool> {
        self.cache.is_cached(url).await
    }
}

/// Read `reader` line-by-line and re-emit each line as a
/// `tracing::trace!` event under one of a fixed set of targets so the
/// subscriber can filter them independently. Runs until EOF; ignores
/// line-level I/O errors so a broken pipe on child exit doesn't
/// propagate. Used both by [`Launcher::launch`] (MC itself) and by
/// the Forge processor pipeline (install-time subprocesses like
/// SpecialSource / installertools).
pub(crate) async fn forward_lines<R: AsyncRead + Unpin>(reader: R, target: &'static str) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // `target:` on the macro form has to be a literal, so we
        // dispatch here per known target.
        match target {
            "minecraft::stdout" => tracing::trace!(target: "minecraft::stdout", "{line}"),
            "minecraft::stderr" => tracing::trace!(target: "minecraft::stderr", "{line}"),
            _ => tracing::trace!(target: "minecraft", "{line}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct LauncherBuilder {
    cache_dir: Option<PathBuf>,
}

impl LauncherBuilder {
    pub fn build(self) -> Launcher {
        let cache_dir = self.cache_dir.unwrap_or_else(|| {
            let project_dirs = ProjectDirs::from("", "", "cart")
                .expect("Could not find valid home directory path");

            project_dirs.cache_dir().to_owned()
        });

        let client = Client::new();
        let cache = Cache::new(cache_dir, client.clone());

        Launcher { cache, client }
    }

    pub fn cache_dir(mut self, cache_dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(cache_dir.into());

        self
    }
}
