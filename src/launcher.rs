mod arguments;
mod cache;
pub mod forge;
mod instance;
mod java;

pub use cache::ModCache;
pub use instance::Instance;
use tempfile::TempDir;
use tokio::{fs, process::Command};

use std::{collections::HashMap, path::PathBuf};

use anyhow::anyhow;
use directories_next::ProjectDirs;
use reqwest::Client;

use crate::api::{
    Endpoint,
    forge::ForgePromotions,
    piston::{Arguments, Version, VersionManifest},
};
use cache::{AssetCache, Cache};

pub struct Launcher {
    cache: Cache,
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
        let cache = Cache::new(cache_dir.to_owned(), client);

        Self { cache }
    }

    pub fn builder() -> LauncherBuilder {
        Default::default()
    }

    /// Assemble the full Java command that would launch `instance`,
    /// including asset/Java/mod resolution and Forge install where
    /// requested, plus the temporary natives directory it references.
    /// Split out from `launch` so tests can spawn on their own timeline
    /// — the `TempDir` must outlive the child process.
    pub async fn build_command(
        &self,
        instance: &Instance,
    ) -> anyhow::Result<(Command, TempDir)> {
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

        // ── Forge ────────────────────────────────────────────────────────────
        let mut resolved_forge_version: Option<String> = None;
        let (client_jar, forge_extra_libraries, forge_main_class, forge_game_args, forge_jvm_args) =
            if let Some(forge_spec) = instance.forge_spec() {
                let forge_version = if matches!(forge_spec, "latest" | "recommended") {
                    let promotions = self
                        .cache
                        .fetch_json::<ForgePromotions>(ForgePromotions::url(), None)
                        .await?;
                    promotions
                        .resolve(version_id, forge_spec)
                        .ok_or_else(|| {
                            anyhow!("no Forge {forge_spec} release for Minecraft {version_id}")
                        })?
                } else {
                    format!("{version_id}-{forge_spec}")
                };

                let result =
                    forge::install(&forge_version, &vanilla_client_jar, &java_path, &self.cache)
                        .await?;

                let fv = result.version;
                let game_args = fv.minecraft_arguments.clone();
                let jvm_args = fv.arguments.jvm.clone();
                let game_args_modern = fv.arguments.game.clone();

                let client_jar = result
                    .patched_client_jar
                    .unwrap_or_else(|| vanilla_client_jar.clone());

                resolved_forge_version = Some(forge_version);

                (
                    client_jar,
                    fv.libraries,
                    Some(fv.main_class),
                    (game_args, game_args_modern),
                    jvm_args,
                )
            } else {
                (vanilla_client_jar, vec![], None, (None, vec![]), vec![])
            };

        let classpath = java::build_class_path(
            &version,
            &client_jar,
            &forge_extra_libraries,
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
                "library_directory",
                forge::local_maven_dir(&self.cache)
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

        match &resolved_forge_version {
            Some(fv) => tracing::info!("launching minecraft {} with forge {fv}", version.id),
            None => tracing::info!("launching minecraft {}", version.id),
        }

        Ok((command, natives_directory))
    }

    /// Assemble the launch command and wait for Minecraft to exit. A
    /// non-zero exit is logged but not surfaced — Minecraft prints
    /// crashes to `logs/latest.log`, and callers of `run` shouldn't get
    /// an error just because the user closed the game.
    pub async fn launch(&self, instance: &Instance) -> anyhow::Result<()> {
        let (mut command, _natives_directory) = self.build_command(instance).await?;

        let status = command.status().await?;
        if !status.success() {
            let log_path = instance.directory().join("logs").join("latest.log");
            tracing::warn!(
                "minecraft exited with {status}; see {}",
                log_path.display()
            );
        }

        Ok(())
    }

    pub fn mod_cache(&self) -> ModCache<'_> {
        ModCache::new(&self.cache)
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
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
        let cache = Cache::new(cache_dir, client);

        Launcher { cache }
    }

    pub fn cache_dir(mut self, cache_dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(cache_dir.into());

        self
    }
}
