//! Fixture-driven serde coverage for the Modrinth API.
//!
//! Fixtures under `tests/fixtures/modrinth/` are real responses trimmed
//! to a handful of versions each (the full JEI/1.20.1/forge list is 137
//! versions and 149 KB — the same shape is exercised by the first five).
//! Two mods are covered because JEI's versions carry no `dependencies`
//! and Tinkers' Construct's carry required ones; the field is
//! `#[serde(default)]` so absence and presence must both work.

use std::path::Path;

use cart::api::modrinth::{DependencyType, Project, Version};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/modrinth")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn project(name: &str) -> Project {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|e| panic!("project {name}: {e}"))
}

fn versions(name: &str) -> Vec<Version> {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|e| panic!("versions {name}: {e}"))
}

#[test]
fn project_deserializes_with_slug_and_title() {
    let jei = project("project_jei.json");
    assert_eq!(jei.slug, "jei");
    assert_eq!(jei.title, "Just Enough Items (JEI)");

    let tc = project("project_tinkers_construct.json");
    assert_eq!(tc.slug, "tinkers-construct");
    assert!(!tc.title.is_empty());
}

#[test]
fn versions_carry_files_and_metadata() {
    let all = versions("versions_jei_forge_1.20.1.json");
    assert!(!all.is_empty(), "no versions in fixture");
    for v in &all {
        assert!(!v.version_number.is_empty(), "empty version_number");
        assert!(!v.files.is_empty(), "{}: no files", v.version_number);
        assert!(
            v.game_versions.iter().any(|g| g == "1.20.1"),
            "{}: fixture is filtered to 1.20.1 but not tagged",
            v.version_number,
        );
        assert!(
            v.loaders.iter().any(|l| l == "forge"),
            "{}: fixture is filtered to forge but not tagged",
            v.version_number,
        );
    }
}

/// `resolve()` picks the file with `primary = true` if any, falling back
/// to the first — so the fixture must contain at least one primary file
/// somewhere. If Modrinth ever stops marking any file primary, this
/// forces a re-check of that fallback.
#[test]
fn at_least_one_primary_file_across_versions() {
    let all = versions("versions_jei_forge_1.20.1.json");
    let any_primary = all.iter().any(|v| v.files.iter().any(|f| f.primary));
    assert!(any_primary, "no version has a primary file");
}

/// `date_published` drives version selection in the loose case
/// (`versions.into_iter().max_by_key(|v| v.date_published)`), so if
/// chrono ever fails to parse Modrinth's timestamps `cart add`/`update`
/// silently pick the wrong version.
#[test]
fn dates_are_parseable_and_strictly_ordered_or_equal() {
    let all = versions("versions_jei_forge_1.20.1.json");
    let dates: Vec<_> = all.iter().map(|v| v.date_published).collect();
    // Fixture is sliced from the head of Modrinth's response, which is
    // newest-first; assert the slice is monotonically non-increasing.
    for pair in dates.windows(2) {
        assert!(pair[0] >= pair[1], "dates out of order: {pair:?}");
    }
}

/// The `dependency_type` enum drives the `cart add` warning about
/// required deps. All four variants must round-trip through their
/// lowercase JSON spelling, and a version in the wild must resolve to
/// `Required` so we know we're exercising the real payload.
#[test]
fn dependency_types_deserialize() {
    for spelling in ["required", "optional", "incompatible", "embedded"] {
        let json = format!("\"{spelling}\"");
        let _: DependencyType = serde_json::from_str(&json).unwrap();
    }

    let all = versions("versions_tinkers_construct_forge_1.20.1.json");
    let all_deps: Vec<_> = all.iter().flat_map(|v| &v.dependencies).collect();
    assert!(!all_deps.is_empty(), "fixture picked a mod with no deps");
    assert!(
        all_deps
            .iter()
            .any(|d| d.dependency_type == DependencyType::Required),
        "expected at least one Required dep"
    );
}

/// `dependencies` is `#[serde(default)]` — versions without the field
/// (JEI in this fixture) must still deserialize into an empty vec, not
/// error out.
#[test]
fn missing_dependencies_field_defaults_to_empty() {
    for v in versions("versions_jei_forge_1.20.1.json") {
        assert!(
            v.dependencies.is_empty(),
            "{}: expected no dependencies",
            v.version_number
        );
    }
}
