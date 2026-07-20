mod arguments;
mod cache;
mod instance;
mod java;

pub use instance::Instance;
use tokio::fs;

use std::{collections::HashMap, path::PathBuf};

use directories_next::ProjectDirs;
use reqwest::Client;

use crate::api::{
    Endpoint,
    piston::{Arguments, Version, VersionManifest},
};
use cache::{AssetCache, Cache, ModCache};

pub struct Launcher {
    cache: Cache,
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

    pub async fn launch(&self, instance: &Instance) -> anyhow::Result<()> {
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

        let version_info = &version_map.get(version_id).expect("unknown version");
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

        let natives_directory = tempfile::tempdir()?;

        let classpath = java::build_class_path(&version, &natives_directory, &self.cache).await?;

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
            (
                "auth_uuid",
                "00000000-0000-0000-0000-000000000000".to_owned(),
            ),
            ("auth_access_token", "0".to_owned()),
            ("user_type", "legacy".to_owned()),
            ("version_type", version.kind.as_ref().to_owned()),
            (
                "natives_directory",
                natives_directory.path().to_string_lossy().into_owned(),
            ),
            ("launcher_name", "cart".to_owned()),
            ("launcher_version", env!("CARGO_PKG_VERSION").to_owned()),
            ("classpath", classpath),
        ]
        .into_iter()
        .collect();

        let mut command = java::java_binary(&java_path);
        command.current_dir(instance.directory());
        command.args(["-Xmx4G", "-Xms1G"]);

        match &version.arguments {
            Arguments::Modern { jvm, game } => {
                command.args(arguments::resolve(jvm, &variables));
                command.arg(&version.main_class);
                command.args(arguments::resolve(game, &variables));
            }
            Arguments::Legacy(minecraft_arguments) => {
                command.args([
                    arguments::substitute("-Djava.library.path=${natives_directory}", &variables),
                    "-cp".to_owned(),
                    arguments::substitute("${classpath}", &variables),
                ]);
                command.arg(&version.main_class);
                command.args(
                    minecraft_arguments
                        .split_ascii_whitespace()
                        .map(|token| arguments::substitute(token, &variables)),
                );
            }
        }

        command.status().await?;

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
