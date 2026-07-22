use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, bail};
use tokio::{fs, process::Command};
use url::Url;
use zip::ZipArchive;

use crate::api::{
    forge::{ForgeVersion, InstallProfile, MavenCoordinate, Processor},
    neoforge,
};

/// The `data` key whose client value is the Maven coord of the patched client
/// JAR produced by the Forge install pipeline (spec 0+).
const PATCHED_DATA_KEY: &str = "PATCHED";

use super::cache::Cache;

pub static FORGE_MAVEN_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://maven.minecraftforge.net/").unwrap());

pub static NEOFORGE_MAVEN_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://maven.neoforged.net/releases/").unwrap());

/// Forge-family installer flavor. Both Forge and NeoForge share the same
/// install pipeline (installer JAR → install_profile.json → processors →
/// patched client + extra libraries + version.json overlay); only the
/// maven base URL, installer path, and version quirks differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgeFlavor {
    Forge,
    NeoForge,
}

impl ForgeFlavor {
    pub fn maven_base_url(&self) -> &'static Url {
        match self {
            Self::Forge => &FORGE_MAVEN_URL,
            Self::NeoForge => &NEOFORGE_MAVEN_URL,
        }
    }

    /// Subdirectory under the cache root where this flavor's local maven
    /// mirror lives. Matches the URL-mirrored layout `Cache::path_from_url`
    /// uses (host + release-repository path for NeoForge).
    fn local_maven_subdir(&self) -> &'static str {
        match self {
            Self::Forge => "maven.minecraftforge.net",
            Self::NeoForge => "maven.neoforged.net/releases",
        }
    }

    /// Installer URL candidates in preference order. Forge has the 1.7.10
    /// doubled-suffix quirk; NeoForge (1.20.2+) has no such variation, so
    /// its candidate list is always a single entry.
    fn installer_url_candidates(&self, version: &str) -> Vec<(String, Url)> {
        match self {
            Self::Forge => crate::api::forge::installer_url_candidates(version),
            Self::NeoForge => {
                vec![(version.to_owned(), neoforge::installer_url(version))]
            }
        }
    }
}

/// Local Maven-style working directory for the install pipeline. This is
/// both where processor outputs land and the `library_directory` that
/// modern (1.17+) Forge/NeoForge JVM args and FMLLoader resolve paths
/// against — so it must be a single directory that also contains every
/// downloaded loader lib.
pub fn local_maven_dir(flavor: ForgeFlavor, cache: &Cache) -> PathBuf {
    cache.directory().join(flavor.local_maven_subdir())
}

pub struct ForgeInstallResult {
    pub version: ForgeVersion,
    /// `None` for legacy Forge (pre-1.13): use the vanilla client JAR as-is.
    pub patched_client_jar: Option<PathBuf>,
    /// The Forge version identifier that actually resolved on the maven —
    /// may differ from the caller's input for 1.7.10 (see
    /// [`installer_url_candidates`] for why).
    pub effective_version: String,
}

/// Downloads and runs the Forge-family installer pipeline for `forge_version`
/// (e.g. `"1.20.1-47.3.12"` for Forge, `"20.4.237"` for NeoForge).  The
/// pipeline is skipped if the patched client JAR already exists in the
/// local cache.
pub async fn install(
    flavor: ForgeFlavor,
    forge_version: &str,
    vanilla_jar: &Path,
    java_path: &Path,
    cache: &Cache,
) -> anyhow::Result<ForgeInstallResult> {
    let (effective_version, installer_path) = {
        let candidates = flavor.installer_url_candidates(forge_version);
        let mut last_err = None;
        let mut winner = None;
        for (candidate_version, url) in candidates {
            match cache.fetch(&url, None).await {
                Ok(path) => {
                    winner = Some((candidate_version, path));
                    break;
                }
                Err(err) => last_err = Some(err),
            }
        }
        winner.ok_or_else(|| {
            let err = last_err.expect("candidates list is non-empty");
            err.context(format!(
                "no Forge installer at any candidate URL for {forge_version}"
            ))
        })?
    };

    let (install_profile, forge_version_manifest) =
        parse_installer(&installer_path).context("failed to parse Forge installer")?;

    let marker = cache
        .directory()
        .join("forge")
        .join(&effective_version)
        .join(".installed");

    // Any library with an empty URL is bundled inside the installer
    // under maven/{artifact.path}. Extract those to the Forge Maven
    // cache path so build_class_path can find them by URL.
    //
    // Walks BOTH install_profile.libraries (needed by the processor
    // pipeline) AND version.json's libraries (needed on the launch
    // classpath) — 1.16.5 has an embedded `forge-<ver>.jar` in the
    // version.json list only, and skipping it made build_class_path
    // 404 upstream where no unclassified forge jar is published.
    for lib in install_profile
        .libraries
        .iter()
        .chain(forge_version_manifest.libraries.iter())
    {
        if let Some(artifact) = &lib.downloads.artifact
            && artifact.url.is_none() {
                let entry_name = format!("maven/{}", artifact.path.display());
                let jar_url = flavor
                    .maven_base_url()
                    .join(&artifact.path.to_string_lossy())
                    .with_context(|| {
                        format!("failed to build Forge Maven URL for: {}", lib.name)
                    })?;
                let cache_path = cache.path_from_url(&jar_url)?;
                if !fs::try_exists(&cache_path).await? {
                    extract_from_installer(&installer_path, &entry_name, &cache_path)?;
                }
            }
    }

    // Legacy Forge (1.7.10 era) bundles the Forge JAR at the root of
    // the installer ZIP as `forge-{version}-universal.jar`, and lists
    // it in versionInfo.libraries as `net.minecraftforge:forge:{version}`
    // with NO `downloads` block. Forge maven publishes only the
    // `-universal` classifier, so the standard "look it up by Maven
    // coordinate" fallback in build_class_path would 404. Extract the
    // universal JAR out of the installer and place it at the unclassified
    // coordinate path, which is what the versionInfo entry resolves to.
    for lib in &forge_version_manifest.libraries {
        if lib.name.starts_with("net.minecraftforge:forge:") && lib.downloads.artifact.is_none() {
            let coord = MavenCoordinate::parse(&lib.name)
                .with_context(|| format!("failed to parse Maven coordinate: {}", lib.name))?;
            let jar_url = flavor
                .maven_base_url()
                .join(&coord.to_path().to_string_lossy())
                .with_context(|| format!("failed to build Forge Maven URL for: {}", lib.name))?;
            let cache_path = cache.path_from_url(&jar_url)?;
            if !fs::try_exists(&cache_path).await? {
                let entry_name = format!("forge-{effective_version}-universal.jar");
                extract_from_installer(&installer_path, &entry_name, &cache_path)?;
            }
        }
    }

    // Legacy Forge (no processors): no client JAR patching needed.
    if install_profile.processors.is_empty() {
        if !fs::try_exists(&marker).await? {
            fs::create_dir_all(marker.parent().unwrap()).await?;
            fs::write(&marker, "").await?;
        }

        return Ok(ForgeInstallResult {
            version: forge_version_manifest,
            patched_client_jar: None,
            effective_version,
        });
    }

    // Modern Forge (1.13+): run the processor pipeline to patch the client JAR.
    let (lib_paths, resolved_data) =
        download_and_resolve(flavor, &install_profile, &installer_path, cache).await?;

    let patched_client_jar = resolved_data
        .get(PATCHED_DATA_KEY)
        .cloned()
        .with_context(|| {
            format!(
                "install_profile.data has no `{PATCHED_DATA_KEY}` entry; \
                 available data keys: {:?}",
                install_profile.data.keys().collect::<Vec<_>>()
            )
        })?;

    if !fs::try_exists(&marker).await? {
        for path in resolved_data.values() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }
        }

        let local_dir = local_maven_dir(flavor, cache);
        run_processors(
            &install_profile.processors,
            &resolved_data,
            &lib_paths,
            &local_dir,
            vanilla_jar,
            &installer_path,
            java_path,
        )
        .await?;

        fs::create_dir_all(marker.parent().unwrap()).await?;
        fs::write(&marker, "").await?;
    }

    Ok(ForgeInstallResult {
        version: forge_version_manifest,
        patched_client_jar: Some(patched_client_jar),
        effective_version,
    })
}

// ── Installer parsing ─────────────────────────────────────────────────────────

fn parse_installer(installer_path: &Path) -> anyhow::Result<(InstallProfile, ForgeVersion)> {
    let file = std::fs::File::open(installer_path)?;
    let mut archive = ZipArchive::new(file)?;

    let install_profile: InstallProfile = {
        let mut entry = archive.by_name("install_profile.json")?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    };

    // Modern format (1.13+) has a separate version.json; legacy embeds versionInfo.
    let forge_version = if let Ok(mut entry) = archive.by_name("version.json") {
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)?
    } else {
        install_profile
            .version_info
            .clone()
            .context("install_profile.json has no versionInfo and version.json is absent")?
    };

    Ok((install_profile, forge_version))
}

// ── Library download & data resolution ───────────────────────────────────────

/// Downloads all libraries listed in the install profile and resolves the `data`
/// map to concrete local paths.  Returns `(lib_paths, resolved_data)` where
/// `lib_paths` maps Maven coordinate names to their cached JAR paths (used to
/// look up processor JARs) and `resolved_data` maps data keys to local paths.
async fn download_and_resolve(
    flavor: ForgeFlavor,
    profile: &InstallProfile,
    installer_path: &Path,
    cache: &Cache,
) -> anyhow::Result<(HashMap<String, PathBuf>, HashMap<String, PathBuf>)> {
    let mut lib_paths: HashMap<String, PathBuf> = HashMap::new();

    for lib in &profile.libraries {
        let Some(artifact) = &lib.downloads.artifact else {
            continue;
        };

        // Downloaded from an explicit URL, or bundled inside the installer
        // under `maven/{artifact.path}` (which we've already extracted to the
        // Forge Maven cache path earlier in `install`).
        let path = if let Some(url) = &artifact.url {
            cache.fetch(url, Some(&artifact.sha1)).await?
        } else {
            let jar_url = flavor
                .maven_base_url()
                .join(&artifact.path.to_string_lossy())
                .with_context(|| format!("failed to build Forge Maven URL for: {}", lib.name))?;
            cache.path_from_url(&jar_url)?
        };

        lib_paths.insert(lib.name.clone(), path);
    }

    let local_dir = local_maven_dir(flavor, cache);
    let mut resolved: HashMap<String, PathBuf> = HashMap::new();

    for (key, entry) in &profile.data {
        let path = resolve_data_value(&entry.client, &lib_paths, installer_path, &local_dir)?;
        resolved.insert(key.clone(), path);
    }

    Ok((lib_paths, resolved))
}

fn resolve_data_value(
    value: &str,
    lib_paths: &HashMap<String, PathBuf>,
    installer_path: &Path,
    local_dir: &Path,
) -> anyhow::Result<PathBuf> {
    if let Some(inner) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        // Maven coordinate — either a downloaded library or a processor output.
        if let Some(path) = lib_paths.get(inner) {
            return Ok(path.clone());
        }
        let coord = MavenCoordinate::parse(inner)?;
        return Ok(local_dir.join(coord.to_path()));
    }

    if let Some(inner_path) = value.strip_prefix('/') {
        // File embedded inside the installer JAR. Scope the extraction
        // path by the installer's filename stem so multiple Forge
        // versions (e.g. 1.16.5-36.2.42 and 1.20.1-47.4.21) don't
        // collide on a shared `/data/client.lzma` path — otherwise the
        // first install's binpatch gets fed to every subsequent install
        // whose data value happens to name the same inner path.
        let installer_key = installer_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown-installer");
        let target = local_dir
            .join("installer-data")
            .join(installer_key)
            .join(inner_path);
        if !target.exists() {
            extract_from_installer(installer_path, inner_path, &target)?;
        }
        return Ok(target);
    }

    Ok(PathBuf::from(value))
}

fn extract_from_installer(
    installer_path: &Path,
    entry_name: &str,
    target: &Path,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(installer_path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut entry = archive.by_name(entry_name)?;

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    std::fs::write(target, &buf)?;

    Ok(())
}

// ── Processor execution ───────────────────────────────────────────────────────

async fn run_processors(
    processors: &[Processor],
    resolved_data: &HashMap<String, PathBuf>,
    lib_paths: &HashMap<String, PathBuf>,
    local_dir: &Path,
    vanilla_jar: &Path,
    installer_path: &Path,
    java_path: &Path,
) -> anyhow::Result<()> {
    for processor in processors {
        // Only run client-side processors.
        if let Some(sides) = &processor.sides
            && !sides.iter().any(|s| s == "client") {
                continue;
            }

        let proc_jar = lib_paths
            .get(&processor.jar)
            .with_context(|| format!("processor JAR not found: {}", processor.jar))?;

        let main_class = read_main_class(proc_jar)?;

        let mut classpath: Vec<String> = vec![proc_jar.to_string_lossy().into_owned()];
        for coord in &processor.classpath {
            let path = lib_paths
                .get(coord)
                .with_context(|| format!("classpath entry not found: {coord}"))?;
            classpath.push(path.to_string_lossy().into_owned());
        }

        let args: Vec<String> = processor
            .args
            .iter()
            .map(|arg| {
                substitute_arg(
                    arg,
                    resolved_data,
                    lib_paths,
                    local_dir,
                    vanilla_jar,
                    installer_path,
                )
            })
            .collect();

        // Ensure the parent dir exists for any `[maven:coord]` output that
        // resolves into the local Maven working directory.
        for arg in &args {
            let path = Path::new(arg);
            if path.starts_with(local_dir)
                && let Some(parent) = path.parent()
            {
                fs::create_dir_all(parent).await?;
            }
        }

        let status = Command::new(java_path.join("bin").join("java"))
            .arg("-cp")
            .arg(classpath.join(":"))
            .arg(&main_class)
            .args(&args)
            .status()
            .await?;

        if !status.success() {
            bail!("Forge processor {} exited with {status}", processor.jar);
        }
    }

    Ok(())
}

fn read_main_class(jar_path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(jar_path)?;
    let mut archive = ZipArchive::new(file)?;

    let mut manifest = archive.by_name("META-INF/MANIFEST.MF")?;
    let mut content = String::new();
    manifest.read_to_string(&mut content)?;

    for line in content.lines() {
        if let Some(class) = line.strip_prefix("Main-Class:") {
            return Ok(class.trim().to_owned());
        }
    }

    bail!("no Main-Class in MANIFEST.MF for {}", jar_path.display())
}

/// Substitutes a single processor argument token.
///
/// * `{KEY}` — replaced by the resolved path for that data key, or one of the
///   built-in variables (`MINECRAFT_JAR`, `SIDE`, `INSTALLER`).
/// * `[maven:coord]` — resolved to the downloaded library path, falling back
///   to the Maven-style path inside `local_dir` (for coords that name a
///   processor output rather than a library).
/// * Anything else is returned unchanged.
fn substitute_arg(
    arg: &str,
    resolved_data: &HashMap<String, PathBuf>,
    lib_paths: &HashMap<String, PathBuf>,
    local_dir: &Path,
    vanilla_jar: &Path,
    installer_path: &Path,
) -> String {
    if let Some(key) = arg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return match key {
            "MINECRAFT_JAR" => vanilla_jar.to_string_lossy().into_owned(),
            "SIDE" => "client".to_owned(),
            "INSTALLER" => installer_path.to_string_lossy().into_owned(),
            _ => resolved_data
                .get(key)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| arg.to_owned()),
        };
    }

    if let Some(inner) = arg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if let Some(path) = lib_paths.get(inner) {
            return path.to_string_lossy().into_owned();
        }
        if let Ok(coord) = MavenCoordinate::parse(inner) {
            return local_dir.join(coord.to_path()).to_string_lossy().into_owned();
        }
    }

    arg.to_owned()
}

