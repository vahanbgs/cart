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

use cart::{Instance, Launcher};

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
    let launcher = Launcher::new();
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
                "{version}: minecraft exited within {watch:?}: {status}\n\
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
