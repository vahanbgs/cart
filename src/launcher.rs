mod cache;
mod instance;

pub use instance::Instance;

use std::{
    collections::HashMap,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use directories_next::ProjectDirs;
use reqwest::Client;
use tempfile::TempDir;
use tokio::{
    fs::{self, File},
    process::Command,
};
use zip::ZipArchive;

use crate::api::{
    Endpoint,
    piston::{
        Action, Argument, ArgumentValue, Arguments, FileSystemEntry, GameJarDownloadOptions,
        JavaDistribution, JavaDistributionManifest, JavaPlatform, JavaVersionComponent,
        NativeClassifier, Os, Rule, Version, VersionManifest,
    },
};
use cache::{AssetCache, Cache, ModCache};

async fn make_executable(path: impl AsRef<Path>) -> anyhow::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o755)).await?;

    Ok(())
}

async fn fetch_version_manifest(cache: &Cache) -> anyhow::Result<VersionManifest> {
    cache.fetch_json(VersionManifest::url(), None).await
}

async fn fetch_java_distribution_manifest(
    cache: &Cache,
) -> anyhow::Result<JavaDistributionManifest> {
    cache
        .fetch_json(JavaDistributionManifest::url(), None)
        .await
}

async fn fetch_java_distribution(
    java_version_component: JavaVersionComponent,
    cache: &Cache,
) -> anyhow::Result<PathBuf> {
    let java_distribution_manifest = fetch_java_distribution_manifest(cache).await?;

    let java_distribution_info =
        &java_distribution_manifest.0[&JavaPlatform::CURRENT][&java_version_component][0];

    let java_distribution = cache
        .fetch_json::<JavaDistribution>(
            &java_distribution_info.manifest.url,
            Some(&java_distribution_info.manifest.sha1),
        )
        .await?;

    let java_distribution_path = cache
        .directory()
        .join("java")
        .join(java_version_component.as_ref());

    for (path, fs_entry) in java_distribution.files {
        match fs_entry {
            FileSystemEntry::File {
                downloads,
                executable,
            } => {
                let source_path = cache
                    .fetch(&downloads.raw.url, Some(&downloads.raw.sha1))
                    .await?;

                let target_path = java_distribution_path.join(path);

                fs::create_dir_all(target_path.parent().unwrap()).await?;

                if !fs::try_exists(&target_path).await? {
                    fs::hard_link(source_path, &target_path).await?;
                }

                if executable {
                    make_executable(target_path).await?;
                }
            }
            _ => {}
        }
    }

    Ok(java_distribution_path)
}

async fn fetch_game_client_jar(
    game_jar_download_options: &GameJarDownloadOptions,
    cache: &Cache,
) -> anyhow::Result<PathBuf> {
    let download_entry = &game_jar_download_options.client;

    cache
        .fetch(&download_entry.url, Some(&download_entry.sha1))
        .await
}

async fn build_class_path(
    version_manifest: &Version,
    natives_directory: &TempDir,
    cache: &Cache,
) -> anyhow::Result<String> {
    let game_client_jar_path = fetch_game_client_jar(&version_manifest.downloads, &cache).await?;

    let mut classpath = vec![game_client_jar_path.to_string_lossy().into_owned()];

    for library_entry in &version_manifest.libraries {
        let mut allow = library_entry.rules.is_none();

        if let Some(rules) = &library_entry.rules {
            allow = rules.is_empty();

            for rule in rules {
                let mut rule_applies = true;

                if let Some(os) = &rule.os {
                    rule_applies &= match os {
                        Os::Arch { arch: _ } => true,
                        Os::Name { name } => name.matches_current_platform(),
                    }
                }

                rule_applies &= matches!(rule.features, None);

                if rule_applies {
                    allow = rule.action == Action::Allow;
                }
            }
        }

        if !allow {
            continue;
        }

        if let Some(artifact) = &library_entry.downloads.artifact {
            let path = cache.fetch(&artifact.url, Some(&artifact.sha1)).await?;
            classpath.push(path.to_string_lossy().into_owned());
        }

        if let Some(native) = &library_entry.downloads.classifiers {
            if let Some(native) = native.get(&NativeClassifier::current()) {
                let jar_path = cache.fetch(&native.url, Some(&native.sha1)).await?;
                let jar_file = File::open(jar_path).await?;
                let mut archive = ZipArchive::new(jar_file.into_std().await)?;

                archive.extract(natives_directory)?;
            }
        }
    }

    Ok(classpath.join(":"))
}

fn substitute(argument_string: &str, variables: &HashMap<&str, String>) -> String {
    let mut result = argument_string.to_owned();

    for (key, value) in variables {
        result = result.replace(&format!("${{{key}}}"), value);
    }

    result
}

fn evaluate_rules(rules: &[Rule]) -> bool {
    let mut allow = rules.is_empty();

    for rule in rules {
        let mut applies = true;

        if let Some(os) = &rule.os {
            applies &= match os {
                Os::Arch { .. } => true,
                Os::Name { name } => name.matches_current_platform(),
            };
        }

        applies &= rule.features.is_none();

        if applies {
            allow = rule.action == Action::Allow;
        }
    }

    allow
}

fn resolve_args(arguments: &[Argument], variables: &HashMap<&str, String>) -> Vec<String> {
    let mut processed_arguments = Vec::new();

    for argument in arguments {
        match argument {
            Argument::Simple(simple_argument) => {
                processed_arguments.push(substitute(simple_argument, variables))
            }
            Argument::Complex { rules, value } if evaluate_rules(rules) => match value {
                ArgumentValue::Simple(simple_argument) => {
                    processed_arguments.push(substitute(simple_argument, variables))
                }
                ArgumentValue::Multiple(arguments) => {
                    processed_arguments.extend(arguments.iter().map(|s| substitute(s, variables)))
                }
            },
            Argument::Complex { .. } => {}
        }
    }

    processed_arguments
}

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
        let version_manifest = fetch_version_manifest(&self.cache).await?;

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
            fetch_java_distribution(version.java_version.component, &self.cache).await?;

        let natives_directory = tempfile::tempdir()?;

        let classpath = build_class_path(&version, &natives_directory, &self.cache).await?;

        fs::create_dir_all(instance.directory()).await?;

        let vars: HashMap<&str, String> = [
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

        let mut command = Command::new(java_path.join("bin").join("java"));
        command.current_dir(instance.directory());
        command.args(["-Xmx4G", "-Xms1G"]);

        match &version.arguments {
            Arguments::Modern { jvm, game } => {
                command.args(resolve_args(jvm, &vars));
                command.arg(&version.main_class);
                command.args(resolve_args(game, &vars));
            }
            Arguments::Legacy(minecraft_arguments) => {
                command.args([
                    substitute("-Djava.library.path=${natives_directory}", &vars),
                    "-cp".to_owned(),
                    substitute("${classpath}", &vars),
                ]);
                command.arg(&version.main_class);
                command.args(
                    minecraft_arguments
                        .split_ascii_whitespace()
                        .map(|token| substitute(token, &vars)),
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
