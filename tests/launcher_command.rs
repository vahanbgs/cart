//! Tier-A launcher tests: assemble the launch `Command` for a specific
//! Minecraft version and assert its shape. No spawn, no display — this
//! catches classpath/main-class/arg-substitution regressions in
//! `Launcher::build_command` before we pay the cost of an actual launch.
//!
//! `#[ignore]`d because `build_command` warms the shared cart cache
//! (`~/.cache/cart/`) on first run — downloads the version manifest,
//! Java runtime, libraries, and assets for the version under test.
//! Once warm, subsequent runs finish in under a second. Invoke with
//! `cargo test --test launcher_command -- --ignored`.

use cart::{Instance, Launcher};

#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_20_1_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.20.1")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    // We should be running the bundled Java from the cache, not a
    // system java on PATH.
    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );

    // Vanilla 1.20.1's main class — if `build_command` ever assembles
    // the wrong one, the JVM crashes at launch with ClassNotFoundException.
    assert!(
        args.iter().any(|a| a == "net.minecraft.client.main.Main"),
        "expected vanilla main class in args:\n{args:#?}"
    );

    // The template vars that MC actually crashes on if left unresolved.
    // Microsoft-account placeholders (`${clientid}`, `${auth_xuid}`) are
    // deliberately left as-is on offline launch and MC tolerates them.
    for var in [
        "${classpath}",
        "${natives_directory}",
        "${assets_root}",
        "${assets_index_name}",
        "${game_directory}",
        "${version_name}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    // Baseline JVM heap args we always add.
    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}
