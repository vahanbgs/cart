//! Fixture-driven serde coverage for Mojang's Piston endpoints.
//!
//! Each fixture under `tests/fixtures/piston/` is a real response from
//! `piston-meta.mojang.com`, checked in so these tests are offline and
//! deterministic. Regenerate by re-running the same URLs (see the fixture
//! header comment in each file's git history) — if a fixture stops
//! deserializing, Mojang changed the schema.

use std::path::Path;

use cart::api::piston::{
    Arguments, AssetManifest, JavaDistributionManifest, JavaPlatform, JavaVersionComponent, Kind,
    Version, VersionManifest,
};

fn fixture(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/piston")
        .join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn load_version(id: &str) -> Version {
    let raw = fixture(&format!("versions/{id}.json"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("deserialize {id}: {e}"))
}

/// The 7 versions we test, spanning every schema inflection Mojang has
/// shipped: pre-launcher era (b1.7, 1.2.5), pre-`arguments` object with
/// modern library rules (1.6.4 – 1.12.2), and modern (1.16.5, 1.20.1).
const VERSION_IDS: &[&str] = &[
    "b1.7", "1.2.5", "1.6.4", "1.7.10", "1.12.2", "1.16.5", "1.20.1",
];

#[test]
fn version_manifest_deserializes_and_indexes_by_id() {
    let manifest: VersionManifest =
        serde_json::from_str(&fixture("version_manifest_v2.json")).unwrap();

    assert!(!manifest.latest_release().is_empty());
    assert!(!manifest.latest_snapshot().is_empty());

    let by_id = manifest.version_map();
    for id in VERSION_IDS {
        assert!(by_id.contains_key(id), "manifest missing {id}");
    }
}

/// Every version JSON we ship must deserialize with the current types.
/// The per-version invariants below cover the shape *differences*; this
/// test guards the base case that they parse at all.
#[test]
fn every_version_json_deserializes() {
    for id in VERSION_IDS {
        let v = load_version(id);
        assert_eq!(v.id, *id);
        assert!(!v.main_class.is_empty(), "{id}: main_class is empty");
        assert!(!v.libraries.is_empty(), "{id}: no libraries");
    }
}

/// Pre-1.13 versions embed launch arguments as a single space-separated
/// string in `minecraftArguments`; 1.13+ split them into a JSON object.
/// Both shapes must map into the same `Arguments` enum.
#[test]
fn arguments_shape_matches_era() {
    for id in ["b1.7", "1.2.5", "1.6.4", "1.7.10", "1.12.2"] {
        let v = load_version(id);
        assert!(
            matches!(v.arguments, Arguments::Legacy(_)),
            "{id}: expected legacy `minecraftArguments` string"
        );
    }
    for id in ["1.16.5", "1.20.1"] {
        let v = load_version(id);
        assert!(
            matches!(v.arguments, Arguments::Modern { .. }),
            "{id}: expected modern `arguments` object"
        );
    }
}

/// `javaVersion` was added around 1.16; older manifests omit it and
/// callers rely on the `Default` impl to fall back to `jre-legacy` +
/// Java 8. This test locks that default in.
#[test]
fn missing_java_version_defaults_to_jre_legacy_java_8() {
    for id in ["b1.7", "1.2.5", "1.6.4", "1.7.10", "1.12.2"] {
        let v = load_version(id);
        assert_eq!(
            v.java_version.component,
            JavaVersionComponent::JreLegacy,
            "{id}: unexpected Java component"
        );
        assert_eq!(v.java_version.major_version, 8, "{id}: unexpected Java major");
    }
}

/// Modern manifests specify Java explicitly. 1.16.5 needs Java 8
/// (component gamma), 1.20.1 needs Java 17 (component delta). If Mojang
/// ever renames the components, `Launcher` picks the wrong runtime.
#[test]
fn modern_versions_declare_java_component_and_major() {
    let v = load_version("1.16.5");
    assert_eq!(v.java_version.major_version, 8);

    let v = load_version("1.20.1");
    assert_eq!(v.java_version.major_version, 17);
    assert_ne!(v.java_version.component, JavaVersionComponent::JreLegacy);
}

/// `type` on both the manifest listing and per-version JSON is the same
/// enum. b1.7 is the pre-release beta ID; the rest are stable releases.
/// This locks the `Kind` mapping in place — a silent variant rename
/// would flip these matches.
#[test]
fn version_kind_matches_expected() {
    assert_eq!(load_version("b1.7").kind, Kind::OldBeta);
    for id in ["1.2.5", "1.6.4", "1.7.10", "1.12.2", "1.16.5", "1.20.1"] {
        assert_eq!(load_version(id).kind, Kind::Release, "{id}");
    }
}

/// Downloads.client is the field `Launcher` follows to fetch the game
/// jar — a real URL with a valid SHA-1 must be present on every version.
#[test]
fn every_version_advertises_a_client_download() {
    for id in VERSION_IDS {
        let v = load_version(id);
        assert_eq!(v.downloads.client.sha1.to_hex().len(), 40, "{id}");
        assert!(v.downloads.client.size > 0, "{id}: zero-byte client");
    }
}

/// Modern asset indexes have no `map_to_resources` field; the
/// `#[serde(default)]` must fill it in as false so the launcher takes
/// the modern code path instead of copying assets into
/// `.minecraft/resources/`.
#[test]
fn modern_asset_index_defaults_map_to_resources_false() {
    let raw = fixture("asset_indexes/5.json");
    let index: AssetManifest = serde_json::from_str(&raw).unwrap();
    assert!(!index.map_to_resources);
    assert!(!index.objects.is_empty());
    // Every entry must have a 40-char SHA-1 — that's what the cache path
    // is built from.
    for (name, obj) in &index.objects {
        assert_eq!(obj.hash.to_hex().len(), 40, "{name:?}");
        assert!(obj.size > 0, "{name:?}: zero-byte object");
    }
}

/// Pre-1.6 asset indexes set `map_to_resources = true`, telling the
/// launcher to symlink/copy assets into the game dir under
/// `resources/<path>` instead of the modern virtual layout.
#[test]
fn legacy_asset_index_carries_map_to_resources_true() {
    let raw = fixture("asset_indexes/pre-1.6.json");
    let index: AssetManifest = serde_json::from_str(&raw).unwrap();
    assert!(index.map_to_resources);
    assert!(!index.objects.is_empty());
}

/// The full Java distribution manifest is `platform → component → [info]`.
/// The `Launcher` looks up `JavaPlatform::CURRENT`, then follows the
/// component the version JSON asks for; the fixture must contain every
/// component enum variant we ship, on the current platform.
#[test]
fn java_distribution_manifest_covers_current_platform() {
    let raw = fixture("java_distribution.json");
    let manifest: JavaDistributionManifest = serde_json::from_str(&raw).unwrap();
    let per_platform = manifest
        .0
        .get(&JavaPlatform::CURRENT)
        .expect("current platform missing from java distribution manifest");
    // Every variant of JavaVersionComponent needs a corresponding entry
    // in the payload — if Mojang renames one, this breaks loudly.
    for component in [
        JavaVersionComponent::JavaRuntimeAlpha,
        JavaVersionComponent::JavaRuntimeBeta,
        JavaVersionComponent::JavaRuntimeDelta,
        JavaVersionComponent::JavaRuntimeEpsilon,
        JavaVersionComponent::JavaRuntimeGamma,
        JavaVersionComponent::JavaRuntimeGammaSnapshot,
        JavaVersionComponent::JreLegacy,
        JavaVersionComponent::MinecraftJavaExe,
    ] {
        assert!(
            per_platform.contains_key(&component),
            "{component:?} missing on {:?}",
            JavaPlatform::CURRENT,
        );
    }
}
