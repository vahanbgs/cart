//! Fixture-driven serde coverage for the CurseForge API.
//!
//! Fixtures under `tests/fixtures/curseforge/` are raw responses from
//! `api.curseforge.com` trimmed only by `pageSize` — every field a real
//! response carries is present, so a rename or new required field on
//! CurseForge's side fails a test here rather than at build time.

use std::path::Path;

use cart::api::curseforge::{File, HashAlgo, Mod, SearchHit};
use serde::Deserialize;

/// Re-declaration of the private `Envelope<T>` in the module. CurseForge
/// wraps every response in `{"data": ...}` and the wrapper isn't
/// exported; the tests need to peel it themselves.
#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/curseforge")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn from_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    serde_json::from_str::<Envelope<T>>(&fixture(name))
        .unwrap_or_else(|e| panic!("deserialize {name}: {e}"))
        .data
}

#[test]
fn mod_deserializes_with_slug_and_name() {
    let mods: Vec<Mod> = from_fixture("search_jei.json");
    let jei = mods
        .into_iter()
        .find(|m| m.slug == "jei")
        .expect("jei in search results");
    assert_eq!(jei.id, 238222);
    assert_eq!(jei.name, "Just Enough Items (JEI)");
}

#[test]
fn files_list_carries_download_metadata() {
    let files: Vec<File> = from_fixture("files_jei_forge_1.20.1.json");
    assert!(!files.is_empty(), "no files in fixture");
    for f in &files {
        assert!(!f.file_name.is_empty(), "file {} has empty name", f.id);
        assert_eq!(
            f.mod_id, 238222,
            "file {} belongs to a different project?",
            f.id
        );
        assert!(
            f.download_url
                .as_ref()
                .map(|u| u.as_str().ends_with(".jar"))
                .unwrap_or(false),
            "file {}: download_url should end in .jar, got {:?}",
            f.id,
            f.download_url,
        );
        assert!(
            f.game_versions.iter().any(|g| g == "1.20.1"),
            "file {}: fixture filtered to 1.20.1 but not tagged",
            f.id
        );
    }
}

/// `latest_file` picks by `file_date`, so if chrono ever fails to parse
/// CurseForge's timestamp format `cart update` silently regresses to
/// picking the wrong file. The fixture is sliced from the head of the
/// response (newest-first) so dates should be non-increasing.
#[test]
fn file_dates_are_parseable_and_monotonic() {
    let files: Vec<File> = from_fixture("files_jei_forge_1.20.1.json");
    for pair in files.windows(2) {
        assert!(
            pair[0].file_date >= pair[1].file_date,
            "dates out of order: {:?} then {:?}",
            pair[0].file_date,
            pair[1].file_date,
        );
    }
}

/// `File::sha1()` walks `hashes` looking for the SHA-1 entry (algo=1).
/// If CurseForge ever changes the numeric mapping or stops returning
/// SHA-1s, the whole content-addressed cache falls back to
/// no-verification and downloads silently succeed with corrupt bytes.
#[test]
fn every_file_has_a_sha1_hash() {
    let files: Vec<File> = from_fixture("files_jei_forge_1.20.1.json");
    for f in &files {
        let sha1 = f
            .sha1()
            .unwrap_or_else(|| panic!("file {} has no SHA-1 hash", f.id));
        assert_eq!(sha1.len(), 40, "file {} SHA-1 wrong length: {sha1:?}", f.id);
        assert!(
            sha1.chars().all(|c| c.is_ascii_hexdigit()),
            "file {} SHA-1 not hex: {sha1:?}",
            f.id
        );
    }
}

/// The single-file endpoint (`/v1/mods/{id}/files/{fileId}`) returns
/// one `File` object in `data`, not an array. `fetch_file` unwraps that
/// shape at build time — if CF ever changes to `[File]` here, we need
/// to know before a launch fails.
#[test]
fn single_file_endpoint_deserializes_into_one_file() {
    let file: File = from_fixture("file_jei_8419086.json");
    assert_eq!(file.id, 8419086);
    assert_eq!(file.mod_id, 238222);
    assert!(file.download_url.is_some());
    assert!(file.sha1().is_some());
}

/// `HashAlgo` values are hard-coded to CurseForge's mapping; asserting
/// the discriminants here catches an accidental reorder of the enum.
#[test]
fn hash_algo_discriminants_match_curseforge() {
    assert_eq!(HashAlgo::Sha1 as u8, 1);
    assert_eq!(HashAlgo::Md5 as u8, 2);
}

/// The full-text `/v1/mods/search` response decodes into `SearchHit`
/// with the render fields the CLI prints (slug, name, summary,
/// downloadCount, authors). Uses a real live response — CurseForge
/// returns two dozen more fields per hit; ignoring them at the serde
/// layer must not fail deserialization.
#[test]
fn search_hits_deserialize_with_render_fields() {
    let hits: Vec<SearchHit> = from_fixture("search_full_text_appleskin.json");
    assert!(!hits.is_empty(), "no hits in fixture");
    for h in &hits {
        assert!(!h.slug.is_empty(), "hit {} has empty slug", h.id);
        assert!(!h.name.is_empty(), "hit {} has empty name", h.id);
        assert!(!h.authors.is_empty(), "hit {} has no authors", h.slug);
        assert!(!h.primary_author().is_empty());
    }
    // At least one hit in the fixture carries the nested `logo.url` we
    // flatten into `logo_url`; without this assertion the deserializer
    // helper could silently `None` out every entry.
    assert!(
        hits.iter().any(|h| h.logo_url.is_some()),
        "no hit had a logo_url — did the `logo` field name or shape change?",
    );
}
