//! Fabric loader install path — the moral analogue of `launcher::forge` but
//! much smaller. Fabric has no installer JAR, no processor pipeline, and no
//! client-JAR patching: given a `(mc, loader)` pair, the meta API hands back
//! a profile JSON listing the extra libraries + main class the JVM needs,
//! and the launcher merges that on top of vanilla.

use anyhow::{Context, anyhow, bail};

use crate::api::{
    Endpoint,
    fabric::{self, LoaderVersions, Profile, ProfileLibrary},
    forge::MavenCoordinate,
    piston::{
        Argument, Arguments, LibraryDownloadEntry, LibraryDownloadOptions, LibraryEntry,
    },
};

use super::{cache::Cache, LoaderSpec};

pub struct FabricInstallResult {
    pub main_class: String,
    pub libraries: Vec<LibraryEntry>,
    pub game_args: Vec<Argument>,
    pub jvm_args: Vec<Argument>,
}

/// Resolve `spec` to a concrete loader, fetch its profile for `mc_version`,
/// and convert the Fabric-shape libraries into piston `LibraryEntry`s so
/// the classpath builder can consume them unchanged.
pub async fn install(
    mc_version: &str,
    spec: &LoaderSpec,
    cache: &Cache,
) -> anyhow::Result<FabricInstallResult> {
    let effective_version = resolve_loader_version(spec, cache).await?;
    tracing::info!("resolved Fabric loader for mc={mc_version} to {effective_version}");

    let profile: Profile = cache
        .fetch_json(&fabric::profile_url(mc_version, &effective_version), None)
        .await
        .with_context(|| {
            format!(
                "failed to fetch Fabric profile for mc={mc_version} loader={effective_version}"
            )
        })?;

    if profile.inherits_from != mc_version {
        // Not fatal today — but a mismatch means we'd try to overlay Fabric's
        // libs onto the wrong vanilla, which produces very confusing
        // classpath errors downstream. Surface it here.
        bail!(
            "Fabric profile inheritsFrom={} doesn't match requested mc_version={mc_version}",
            profile.inherits_from
        );
    }

    let libraries = profile
        .libraries
        .iter()
        .map(profile_library_to_entry)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let (game_args, jvm_args) = match profile.arguments {
        Arguments::Modern { game, jvm } => (game, jvm),
        // Fabric has always used the modern Arguments shape. If this ever
        // fires it's a Fabric API change, not something to recover from.
        Arguments::Legacy(_) => bail!("Fabric profile carried legacy `minecraftArguments`"),
    };

    Ok(FabricInstallResult {
        main_class: profile.main_class,
        libraries,
        game_args,
        jvm_args,
    })
}

/// `LoaderSpec::Latest` picks the newest `stable=true` entry from the loader
/// listing. `Pinned` passes through verbatim. `Recommended` is a Forge
/// channel — Fabric has no analogue and the manifest parser rejects it, but
/// the branch here is defense-in-depth in case a caller constructs a
/// `Loader` value programmatically.
async fn resolve_loader_version(spec: &LoaderSpec, cache: &Cache) -> anyhow::Result<String> {
    match spec {
        LoaderSpec::Pinned(v) => Ok(v.clone()),
        LoaderSpec::Recommended => {
            bail!("Fabric has no `recommended` channel; use `latest` or a pinned version")
        }
        LoaderSpec::Latest => {
            let list: LoaderVersions = cache.fetch_json(LoaderVersions::url(), None).await?;
            list.0
                .into_iter()
                .find(|v| v.stable)
                .map(|v| v.version)
                .ok_or_else(|| anyhow!("no stable Fabric loader in listing"))
        }
    }
}

/// Convert Fabric's flat `{name, url (base), sha1}` library shape to piston's
/// nested `LibraryEntry`. If `sha1` is present we populate
/// `downloads.artifact` so `build_class_path`'s modern branch verifies the
/// digest; otherwise we leave `downloads.artifact` empty and expose the base
/// URL via `LibraryEntry::url`, and the legacy branch reconstructs the full
/// URL from the Maven coordinate.
fn profile_library_to_entry(lib: &ProfileLibrary) -> anyhow::Result<LibraryEntry> {
    let coord = MavenCoordinate::parse(&lib.name)
        .with_context(|| format!("Fabric library has invalid Maven coord: {}", lib.name))?;
    let path = coord.to_path();

    let artifact = if let Some(sha1) = &lib.sha1 {
        let full_url = lib
            .url
            .join(&path.to_string_lossy())
            .with_context(|| format!("failed to build URL for Fabric library: {}", lib.name))?;
        Some(LibraryDownloadEntry {
            path: path.clone(),
            sha1: sha1.clone(),
            size: lib.size.unwrap_or(0),
            url: Some(full_url),
        })
    } else {
        None
    };

    Ok(LibraryEntry {
        downloads: LibraryDownloadOptions {
            artifact,
            classifiers: None,
        },
        extract: None,
        name: lib.name.clone(),
        natives: None,
        rules: None,
        // Always keep the base URL as the legacy-branch fallback; it's a
        // no-op when `downloads.artifact` is present.
        url: Some(lib.url.clone()),
        clientreq: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    fn base() -> Url {
        Url::parse("https://maven.fabricmc.net/").unwrap()
    }

    #[test]
    fn with_sha1_produces_artifact_with_full_url_and_digest() {
        let lib = ProfileLibrary {
            name: "org.ow2.asm:asm:9.10.1".to_owned(),
            url: base(),
            sha1: Some(crate::Sha1Digest::from_bytes([0xab; 20])),
            size: Some(126151),
        };

        let entry = profile_library_to_entry(&lib).unwrap();
        let artifact = entry
            .downloads
            .artifact
            .expect("sha1 present → artifact populated");
        assert_eq!(
            artifact.url.unwrap().as_str(),
            "https://maven.fabricmc.net/org/ow2/asm/asm/9.10.1/asm-9.10.1.jar",
        );
        assert_eq!(artifact.size, 126151);
    }

    /// Fabric-loader/intermediary self-refs ship without sha1. The
    /// `LibraryEntry` still needs to be usable, so the legacy branch of
    /// `build_class_path` picks it up via `LibraryEntry::url`.
    #[test]
    fn without_sha1_leaves_artifact_empty_and_keeps_base_url() {
        let lib = ProfileLibrary {
            name: "net.fabricmc:fabric-loader:0.19.3".to_owned(),
            url: base(),
            sha1: None,
            size: None,
        };
        let entry = profile_library_to_entry(&lib).unwrap();
        assert!(entry.downloads.artifact.is_none());
        assert_eq!(entry.url.unwrap(), base());
    }
}
