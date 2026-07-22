//! Fixture-driven serde coverage for the Fabric meta API.
//!
//! Fixtures under `tests/fixtures/fabric/` are trimmed real responses:
//! the loader-versions list is capped at the first 5 entries (the API
//! returns 250+ but the same shape is exercised by any 5), and the
//! profile JSON is one real `(mc, loader)` response captured verbatim.

use std::path::Path;

use cart::api::fabric::{GameVersions, LoaderVersions, Profile};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fabric")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn loader_versions_deserializes_all_five_entries() {
    let list: LoaderVersions = serde_json::from_str(&fixture("loader_versions.json")).unwrap();
    assert_eq!(list.0.len(), 5);
}

/// `stable` is the discriminator `LoaderSpec::Latest` will filter by;
/// the fixture must include at least one stable and one non-stable entry
/// so the filter has something to prove.
#[test]
fn loader_versions_carries_the_stable_flag() {
    let list: LoaderVersions = serde_json::from_str(&fixture("loader_versions.json")).unwrap();
    assert!(list.0.iter().any(|v| v.stable));
    assert!(list.0.iter().any(|v| !v.stable));
}

#[test]
fn profile_carries_main_class_and_inherits_from() {
    let profile: Profile = serde_json::from_str(&fixture("profile_1_20_1_0_19_3.json")).unwrap();
    assert_eq!(profile.inherits_from, "1.20.1");
    assert_eq!(
        profile.main_class,
        "net.fabricmc.loader.impl.launch.knot.KnotClient"
    );
}

/// `contains` is the load-bearing helper `cart init` uses to decide
/// whether Fabric belongs in the loader menu. Cover both hit and miss.
#[test]
fn game_versions_membership_check() {
    let list: GameVersions = serde_json::from_str(&fixture("game_versions.json")).unwrap();
    assert!(list.contains("1.20.1"));
    assert!(list.contains("1.14"));
    assert!(!list.contains("b1.7.3"));
}

/// The library shape has one subtlety: Fabric's own self-references
/// (`fabric-loader`, `intermediary`) omit `sha1`, but the ASM libs
/// carry it. Both cases must round-trip.
#[test]
fn profile_library_sha1_is_optional() {
    let profile: Profile = serde_json::from_str(&fixture("profile_1_20_1_0_19_3.json")).unwrap();

    let with_sha = profile
        .libraries
        .iter()
        .find(|l| l.name.starts_with("org.ow2.asm:asm:"))
        .expect("asm library present in fixture");
    assert!(
        with_sha.sha1.is_some(),
        "expected sha1 on {}",
        with_sha.name
    );

    let without_sha = profile
        .libraries
        .iter()
        .find(|l| l.name.starts_with("net.fabricmc:fabric-loader:"))
        .expect("fabric-loader self-ref present in fixture");
    assert!(
        without_sha.sha1.is_none(),
        "expected no sha1 on {}",
        without_sha.name
    );
}
