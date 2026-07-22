//! Tier-B launch smoke: spawn Minecraft, wait for it to log the
//! `LWJGL Version` marker (proof that classpath, natives, and JVM args
//! all resolved and MC's own init got past LWJGL setup), then confirm
//! the process is still alive after a short grace. This is the "did I
//! break something that only shows up at runtime" tier — classpath
//! ordering bugs, missing natives, wrong Java major, unresolved
//! templates that MC can't tolerate.
//!
//! Requires a real display — MC opens actual game windows. Serialize
//! with `--test-threads=1` unless you enjoy 16 MC windows fighting for
//! focus at once:
//!     cargo test --test launcher_launch -- --ignored --test-threads=1
//!
//! Do NOT try to run these under Xvfb, Wayland-headless shims, VNC, or
//! any other virtual display — LWJGL/GLFW's interaction with real
//! graphics stacks is fragile and the failure modes (software GL
//! fallback, missing extensions, GLX version mismatches) look like
//! cart bugs but aren't. If you can't launch MC visually on the
//! machine, skip these tests.
//!
//! Pass condition ladder:
//! 1. Fast path — the `LWJGL Version` substring appears in
//!    `logs/latest.log` (universal across every MC era that uses
//!    log4j, i.e. ~1.7+), THEN the process stays alive for a short
//!    grace to catch immediate post-init crashes like GLFW window
//!    creation failures. Typically fires around 4-7s in on a warm
//!    cache — much faster than the previous unconditional wait.
//! 2. Fallback — the process stays alive for
//!    [`FALLBACK_ALIVE_THRESHOLD`] without the marker appearing.
//!    Preserves the old "alive after 15s = pass" behavior for
//!    legacy MC (b1.7, 1.2.5, 1.6.4) which predates the log4j
//!    logs/latest.log path.
//!
//! Fail condition: the child exits at any point before a pass
//! condition trips. The last 50 lines of `logs/latest.log` are
//! tailed into the panic message to point at the root cause.

use std::{path::Path, time::Duration};

use cart::{Instance, Launcher, Loader, LoaderKind, LoaderSpec};

mod common;

/// Substring that appears in every log4j-era MC launch log once LWJGL
/// has been loaded and reported its version — proof that classpath,
/// natives, and JVM args all resolved.
const INIT_MARKER: &str = "LWJGL Version";

/// After seeing the marker, confirm the process stays alive this long
/// before passing. Catches immediate post-init crashes (GLFW window
/// creation, mod loader init failures).
const POST_MARKER_GRACE: Duration = Duration::from_secs(2);

/// If the process is still alive after this long without the marker,
/// pass anyway — this is the pre-log4j fallback for legacy MC
/// versions that never write `logs/latest.log`. Matches the pre-marker
/// behavior of unconditionally waiting 15s.
const FALLBACK_ALIVE_THRESHOLD: Duration = Duration::from_secs(15);

/// How often we re-scan the log file and re-check child liveness.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

async fn read_log_tail(game_dir: &Path) -> String {
    match tokio::fs::read_to_string(game_dir.join("logs/latest.log")).await {
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(50);
            lines[start..].join("\n")
        }
        Err(_) => String::from("<no logs/latest.log — MC crashed before creating it>"),
    }
}

/// Spawn the assembled launch command for `version` and assert MC gets
/// past init and stays alive.
async fn assert_launches_cleanly(version: &str) {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version(version)
        .build(game_dir.path().to_path_buf());
    spawn_and_watch(instance, game_dir, version).await;
}

/// Loader variant of `assert_launches_cleanly`. Kept separate so the
/// existing vanilla callsites don't need to grow an `Option<Loader>`
/// argument — inline duplication is fine while the shape settles.
async fn assert_launches_with_loader_cleanly(version: &str, loader: Loader) {
    let game_dir = tempfile::tempdir().unwrap();
    let label = format!(
        "{version}+{kind}:{spec}",
        kind = match loader.kind {
            LoaderKind::Fabric => "fabric",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
        },
        spec = match &loader.spec {
            LoaderSpec::Latest => "latest".to_owned(),
            LoaderSpec::Recommended => "recommended".to_owned(),
            LoaderSpec::Pinned(v) => v.clone(),
        }
    );
    let instance = Instance::builder()
        .version(version)
        .loader(loader)
        .build(game_dir.path().to_path_buf());
    spawn_and_watch(instance, game_dir, &label).await;
}

async fn spawn_and_watch(instance: Instance, game_dir: tempfile::TempDir, label: &str) {
    let launcher = Launcher::builder().cache_dir(common::cache_dir()).build();
    let (mut command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    // If the test panics or is aborted, don't leave a Minecraft process
    // orphaned running against the user's display.
    command.kill_on_drop(true);

    let mut child = command.spawn().expect("spawn minecraft");
    let log_path = game_dir.path().join("logs/latest.log");
    let start = tokio::time::Instant::now();
    let mut marker_seen_at: Option<tokio::time::Instant> = None;

    let outcome: Result<(), String> = loop {
        tokio::select! {
            result = child.wait() => {
                let status = result.expect("wait on child");
                let log_tail = read_log_tail(game_dir.path()).await;
                break Err(format!(
                    "{label}: minecraft exited before init completed: {status}\n\
                     --- logs/latest.log tail ---\n{log_tail}"
                ));
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {
                if marker_seen_at.is_none()
                    && let Ok(text) = tokio::fs::read_to_string(&log_path).await
                    && text.contains(INIT_MARKER)
                {
                    marker_seen_at = Some(tokio::time::Instant::now());
                }
                if let Some(seen) = marker_seen_at
                    && seen.elapsed() >= POST_MARKER_GRACE
                {
                    break Ok(());
                }
                if start.elapsed() >= FALLBACK_ALIVE_THRESHOLD {
                    break Ok(());
                }
            }
        }
    };

    // Always reap the child, whether we passed or failed.
    let _ = child.kill().await;
    let _ = child.wait().await;

    if let Err(msg) = outcome {
        panic!("{msg}");
    }
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_vanilla() {
    assert_launches_cleanly("1.12.2").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_vanilla() {
    assert_launches_cleanly("1.20.1").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_vanilla() {
    assert_launches_cleanly("1.16.5").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_7_10_vanilla() {
    assert_launches_cleanly("1.7.10").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_6_4_vanilla() {
    assert_launches_cleanly("1.6.4").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_2_5_vanilla() {
    assert_launches_cleanly("1.2.5").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_b1_7_vanilla() {
    assert_launches_cleanly("b1.7").await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_forge_recommended() {
    assert_launches_with_loader_cleanly("1.12.2", Loader::forge(LoaderSpec::Recommended)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_forge_latest() {
    assert_launches_with_loader_cleanly("1.12.2", Loader::forge(LoaderSpec::Latest)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_forge_recommended() {
    assert_launches_with_loader_cleanly("1.20.1", Loader::forge(LoaderSpec::Recommended)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_forge_latest() {
    assert_launches_with_loader_cleanly("1.20.1", Loader::forge(LoaderSpec::Latest)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_forge_recommended() {
    assert_launches_with_loader_cleanly("1.16.5", Loader::forge(LoaderSpec::Recommended)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_forge_latest() {
    assert_launches_with_loader_cleanly("1.16.5", Loader::forge(LoaderSpec::Latest)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_21_1_neoforge_latest() {
    assert_launches_with_loader_cleanly("1.21.1", Loader::neoforge(LoaderSpec::Latest)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_fabric_latest() {
    assert_launches_with_loader_cleanly("1.20.1", Loader::fabric(LoaderSpec::Latest)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_7_10_forge_recommended() {
    assert_launches_with_loader_cleanly("1.7.10", Loader::forge(LoaderSpec::Recommended)).await;
}
