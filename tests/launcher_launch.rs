//! Tier-B launch smoke: spawn Minecraft, wait N seconds, and assert
//! it didn't exit early. This is the "did I break something that only
//! shows up at runtime" tier — classpath issues, missing natives,
//! wrong Java major, unresolved template that MC can't tolerate.
//!
//! Requires a display. Under this project's Nix devshell run with:
//!     xvfb-run cargo test --test launcher_launch -- --ignored
//! Or against your real X server (opens a real MC window):
//!     cargo test --test launcher_launch -- --ignored
//!
//! Under Xvfb specifically, LWJGL may need a software GL fallback:
//!     LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -s "-screen 0 1024x768x24" \
//!         cargo test --test launcher_launch -- --ignored
//!
//! Once MC gets past init it never exits on its own — it sits at the
//! main menu until closed. So an exit inside the observation window
//! means a crash, and staying alive to the end of the window means we
//! made it through boot cleanly.

use std::{path::Path, time::Duration};

use cart::{Instance, Launcher, Loader, LoaderKind, LoaderSpec};

mod common;

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

/// Spawn the assembled launch command for `version` and assert it's
/// still running after `watch`. On early exit, tail `latest.log` into
/// the panic message so the failure points at the root cause.
async fn assert_launches_cleanly(version: &str, watch: Duration) {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version(version)
        .build(game_dir.path().to_path_buf());
    spawn_and_watch(instance, game_dir, version, watch).await;
}

/// Loader variant of `assert_launches_cleanly`. Kept separate so the
/// existing vanilla callsites don't need to grow an `Option<Loader>`
/// argument — inline duplication is fine while the shape settles.
async fn assert_launches_with_loader_cleanly(version: &str, loader: Loader, watch: Duration) {
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
    spawn_and_watch(instance, game_dir, &label, watch).await;
}

async fn spawn_and_watch(
    instance: Instance,
    game_dir: tempfile::TempDir,
    label: &str,
    watch: Duration,
) {
    let launcher = Launcher::builder().cache_dir(common::cache_dir()).build();
    let (mut command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    // If the test panics or is aborted, don't leave a Minecraft process
    // orphaned running against the user's display.
    command.kill_on_drop(true);

    let mut child = command.spawn().expect("spawn minecraft");

    tokio::select! {
        result = child.wait() => {
            let status = result.expect("wait on child");
            let log_tail = read_log_tail(game_dir.path()).await;
            panic!(
                "{label}: minecraft exited within {watch:?}: {status}\n\
                 --- logs/latest.log tail ---\n{log_tail}"
            );
        }
        _ = tokio::time::sleep(watch) => {
            // Still running — that's the pass condition. Kill it and
            // reap to keep the test env tidy.
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_vanilla() {
    assert_launches_cleanly("1.12.2", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_vanilla() {
    assert_launches_cleanly("1.20.1", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_vanilla() {
    assert_launches_cleanly("1.16.5", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_7_10_vanilla() {
    assert_launches_cleanly("1.7.10", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_6_4_vanilla() {
    assert_launches_cleanly("1.6.4", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_2_5_vanilla() {
    assert_launches_cleanly("1.2.5", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_b1_7_vanilla() {
    assert_launches_cleanly("b1.7", Duration::from_secs(15)).await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_forge_recommended() {
    assert_launches_with_loader_cleanly(
        "1.12.2",
        Loader::forge(LoaderSpec::Recommended),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_12_2_forge_latest() {
    assert_launches_with_loader_cleanly(
        "1.12.2",
        Loader::forge(LoaderSpec::Latest),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_forge_recommended() {
    assert_launches_with_loader_cleanly(
        "1.20.1",
        Loader::forge(LoaderSpec::Recommended),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_forge_latest() {
    assert_launches_with_loader_cleanly(
        "1.20.1",
        Loader::forge(LoaderSpec::Latest),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_forge_recommended() {
    assert_launches_with_loader_cleanly(
        "1.16.5",
        Loader::forge(LoaderSpec::Recommended),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_16_5_forge_latest() {
    assert_launches_with_loader_cleanly(
        "1.16.5",
        Loader::forge(LoaderSpec::Latest),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_21_1_neoforge_latest() {
    assert_launches_with_loader_cleanly(
        "1.21.1",
        Loader::neoforge(LoaderSpec::Latest),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_20_1_fabric_latest() {
    assert_launches_with_loader_cleanly(
        "1.20.1",
        Loader::fabric(LoaderSpec::Latest),
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
#[ignore = "spawns Minecraft — needs a display (see file header)"]
async fn launches_1_7_10_forge_recommended() {
    assert_launches_with_loader_cleanly(
        "1.7.10",
        Loader::forge(LoaderSpec::Recommended),
        Duration::from_secs(15),
    )
    .await;
}
