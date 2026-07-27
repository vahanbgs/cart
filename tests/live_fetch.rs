//! Live end-to-end fetches against Piston and Modrinth, gated behind
//! `#[ignore]` so `cargo test` stays offline and fast. Run manually with
//! `cargo test -- --ignored` when you suspect a fixture has aged out or
//! before a release.
//!
//! Assertions are intentionally shape-only: "still deserializes with the
//! current types." The checked-in fixtures under `tests/fixtures/` do
//! the field-level locking.

use cart::api::{
    Endpoint, curseforge, modrinth,
    piston::{
        AssetManifest, JavaDistributionManifest, JavaPlatform, JavaVersionComponent, Version,
        VersionManifest,
    },
};
use reqwest::Client;

async fn fetch_json<T: serde::de::DeserializeOwned>(client: &Client, url: &url::Url) -> T {
    let text = client
        .get(url.clone())
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"))
        .error_for_status()
        .unwrap_or_else(|e| panic!("GET {url}: {e}"))
        .text()
        .await
        .unwrap_or_else(|e| panic!("body {url}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("deserialize {url}: {e}"))
}

#[tokio::test]
#[ignore = "hits the network"]
async fn piston_version_manifest_deserializes_live() {
    let client = Client::new();
    let manifest: VersionManifest = fetch_json(&client, VersionManifest::url()).await;
    assert!(!manifest.latest_release().is_empty());
    assert!(!manifest.versions().is_empty());
}

/// Follows URLs from the live version manifest so this stays valid even
/// after Mojang re-signs individual version JSONs. Covers the same 7
/// versions as the checked-in fixtures; if a real regression happens
/// only on one era, the per-id assert_eq points at it.
#[tokio::test]
#[ignore = "hits the network"]
async fn piston_all_fixture_versions_still_deserialize() {
    let client = Client::new();
    let manifest: VersionManifest = fetch_json(&client, VersionManifest::url()).await;
    let by_id = manifest.version_map();

    for id in [
        "b1.7", "1.2.5", "1.6.4", "1.7.10", "1.12.2", "1.16.5", "1.20.1",
    ] {
        let info = by_id
            .get(id)
            .unwrap_or_else(|| panic!("live manifest missing {id}"));
        let v: Version = fetch_json(&client, &info.url).await;
        assert_eq!(v.id, id);
    }
}

#[tokio::test]
#[ignore = "hits the network"]
async fn piston_java_distribution_manifest_deserializes_live() {
    let client = Client::new();
    let manifest: JavaDistributionManifest =
        fetch_json(&client, JavaDistributionManifest::url()).await;
    let per_platform = manifest
        .0
        .get(&JavaPlatform::CURRENT)
        .expect("current platform missing");
    assert!(per_platform.contains_key(&JavaVersionComponent::JreLegacy));
}

/// Follow the modern 1.20.1 asset index URL and deserialize its
/// `AssetManifest`. Doesn't assert on `objects` — the count changes
/// between snapshots and isn't what we're guarding against.
#[tokio::test]
#[ignore = "hits the network"]
async fn piston_modern_asset_index_deserializes_live() {
    let client = Client::new();
    let manifest: VersionManifest = fetch_json(&client, VersionManifest::url()).await;
    let by_id = manifest.version_map();
    let info = by_id.get("1.20.1").expect("live manifest missing 1.20.1");
    let version: Version = fetch_json(&client, &info.url).await;
    let index: AssetManifest = fetch_json(&client, &version.asset_index.url).await;
    assert!(!index.objects.is_empty());
    assert!(!index.map_to_resources);
}

#[tokio::test]
#[ignore = "hits the network"]
async fn modrinth_resolve_still_returns_a_file() {
    let client = Client::new();
    let resolved = modrinth::resolve(&client, "jei", None, "1.20.1", "forge")
        .await
        .unwrap();
    assert_eq!(resolved.project_slug, "jei");
    assert!(!resolved.version_number.is_empty());
    assert!(resolved.file.url.as_str().ends_with(".jar"));
}

#[tokio::test]
#[ignore = "hits the network"]
async fn modrinth_search_still_returns_hits() {
    let client = Client::new();
    let hits = modrinth::search(&client, "appleskin", 3, "1.20.1", Some("forge"))
        .await
        .unwrap();
    assert!(!hits.is_empty(), "search returned no hits");
    assert!(
        hits.iter().any(|h| h.slug == "appleskin"),
        "expected appleskin in top-3 hits: got {:?}",
        hits.iter().map(|h| &h.slug).collect::<Vec<_>>()
    );
}

/// CurseForge live-fetch — requires `CURSEFORGE_API_KEY`. Skipped with
/// a stderr note when the key isn't set so it's usable in dev shells
/// where CF creds aren't provisioned, without silently masking real
/// regressions when they are.
#[tokio::test]
#[ignore = "hits the network"]
async fn curseforge_search_still_returns_hits() {
    let Ok(key) = std::env::var("CURSEFORGE_API_KEY") else {
        eprintln!("skipping: CURSEFORGE_API_KEY not set");
        return;
    };
    let client = curseforge::client(&key).expect("build cf client");
    let hits = curseforge::search(
        &client,
        "just enough items",
        5,
        "1.20.1",
        Some(curseforge::LoaderType::Forge),
    )
    .await
    .expect("cf search");
    assert!(!hits.is_empty(), "search returned no hits");
    for h in &hits {
        assert!(!h.slug.is_empty(), "hit {} has empty slug", h.id);
        assert!(
            !h.primary_author().is_empty(),
            "hit {} has no author",
            h.slug
        );
    }
}
