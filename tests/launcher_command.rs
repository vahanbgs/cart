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

/// 1.12.2 is pre-1.13 so it goes through the `Arguments::Legacy` branch
/// of `build_command` — a different code path from 1.20.1's
/// `Arguments::Modern`. Same shape checks apply: the JVM must see a
/// classpath, natives path, resolved asset/game dirs, and the vanilla
/// main class.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_12_2_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.12.2")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.client.main.Main"),
        "expected vanilla main class in args:\n{args:#?}"
    );

    // Legacy versions build `-cp <classpath>` and
    // `-Djava.library.path=<natives>` manually rather than substituting
    // into a template array, so the "critical template" check here
    // primarily proves that path went through `arguments::substitute`
    // instead of leaving `${classpath}` literal.
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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.12.2 with `forge = "recommended"` — exercises the full Forge
/// resolution path (promotions API → installer → local maven layout)
/// on top of the Legacy-arguments launcher code. Main class is still
/// launchwrapper's; Forge injects itself via `--tweakClass
/// net.minecraftforge.fml.common.launcher.FMLTweaker` in the game
/// args, which is the assertion that pins Forge actually got wired.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer"]
async fn build_command_1_12_2_forge_recommended_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.12.2")
        .forge_spec("recommended")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.launchwrapper.Launch"),
        "expected launchwrapper main class in args:\n{args:#?}"
    );

    // The `--tweakClass <FMLTweaker>` pair is Forge's actual entry
    // point on 1.12.2 — launchwrapper delegates to it. If Forge's
    // `minecraftArguments` are ever dropped instead of merged with
    // vanilla's, this pair goes missing and mods never load.
    let tweak_index = args
        .iter()
        .position(|a| a == "--tweakClass")
        .expect("expected --tweakClass in args");
    assert_eq!(
        args.get(tweak_index + 1).map(String::as_str),
        Some("net.minecraftforge.fml.common.launcher.FMLTweaker"),
        "wrong tweakClass value in args:\n{args:#?}"
    );

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

    // Forge for 1.12.2 ships specific classpath entries — proving at
    // least one Forge jar is on the classpath separates a real Forge
    // launch from vanilla-with-forge-flag-ignored.
    let classpath_index = args
        .iter()
        .position(|a| a == "-cp")
        .expect("expected -cp in args");
    let classpath = args
        .get(classpath_index + 1)
        .expect("expected classpath value after -cp");
    assert!(
        classpath.contains("forge"),
        "no forge-named entry on classpath — Forge libraries missing?\n{classpath}"
    );

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.12.2 with `forge = "latest"` — same shape as the `recommended`
/// test but pins the `latest` promotion channel, which today resolves
/// to a different build (14.23.5.2864 vs. 14.23.5.2859). Guards against
/// the two channels ever diverging in ways that break assembly.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer"]
async fn build_command_1_12_2_forge_latest_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.12.2")
        .forge_spec("latest")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.launchwrapper.Launch"),
        "expected launchwrapper main class in args:\n{args:#?}"
    );

    let tweak_index = args
        .iter()
        .position(|a| a == "--tweakClass")
        .expect("expected --tweakClass in args");
    assert_eq!(
        args.get(tweak_index + 1).map(String::as_str),
        Some("net.minecraftforge.fml.common.launcher.FMLTweaker"),
        "wrong tweakClass value in args:\n{args:#?}"
    );

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

    let classpath_index = args
        .iter()
        .position(|a| a == "-cp")
        .expect("expected -cp in args");
    let classpath = args
        .get(classpath_index + 1)
        .expect("expected classpath value after -cp");
    assert!(
        classpath.contains("forge"),
        "no forge-named entry on classpath — Forge libraries missing?\n{classpath}"
    );

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.20.1 with `forge = "recommended"` — modern-era Forge. Main
/// class is `cpw.mods.bootstraplauncher.BootstrapLauncher`, the
/// launcher shells into it via a JPMS module path (`-p`) rather
/// than launchwrapper. Forge's `Arguments::Modern` game arm carries
/// `--launchTarget forgeclient` which is Forge's actual entry hook.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer + processor pipeline"]
async fn build_command_1_20_1_forge_recommended_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.20.1")
        .forge_spec("recommended")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter()
            .any(|a| a == "cpw.mods.bootstraplauncher.BootstrapLauncher"),
        "expected bootstraplauncher main class in args:\n{args:#?}"
    );

    // `--launchTarget forgeclient` is the argument pair Forge appends
    // to select its client entrypoint inside BootstrapLauncher. If
    // Forge's modern-era game args ever fail to merge into the vanilla
    // list, this pair goes missing and BootstrapLauncher errors out
    // with "unknown target".
    let target_index = args
        .iter()
        .position(|a| a == "--launchTarget")
        .expect("expected --launchTarget in args");
    assert_eq!(
        args.get(target_index + 1).map(String::as_str),
        Some("forgeclient"),
        "wrong launchTarget value in args:\n{args:#?}"
    );

    // Modern Forge uses the JPMS module path — the `-p <path>` pair
    // must be present or BootstrapLauncher can't resolve its modules.
    // `${library_directory}` and `${classpath_separator}` also get
    // substituted into the module path, so any unresolved template
    // here is fatal.
    let module_path_index = args
        .iter()
        .position(|a| a == "-p")
        .expect("expected -p (module path) in args");
    let module_path = args
        .get(module_path_index + 1)
        .expect("expected module path value after -p");
    assert!(
        !module_path.contains("${"),
        "unresolved template in module path: {module_path}"
    );
    assert!(
        module_path.contains("bootstraplauncher"),
        "module path missing bootstraplauncher entry:\n{module_path}"
    );

    for var in [
        "${classpath}",
        "${natives_directory}",
        "${assets_root}",
        "${assets_index_name}",
        "${game_directory}",
        "${version_name}",
        "${library_directory}",
        "${classpath_separator}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.20.1 with `forge = "latest"` — same shape as the `recommended`
/// test but pins the `latest` channel, which today resolves to a
/// different build (47.4.22 vs. 47.4.10). Guards the processor
/// pipeline against regressions specific to the newer build.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer + processor pipeline"]
async fn build_command_1_20_1_forge_latest_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.20.1")
        .forge_spec("latest")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter()
            .any(|a| a == "cpw.mods.bootstraplauncher.BootstrapLauncher"),
        "expected bootstraplauncher main class in args:\n{args:#?}"
    );

    let target_index = args
        .iter()
        .position(|a| a == "--launchTarget")
        .expect("expected --launchTarget in args");
    assert_eq!(
        args.get(target_index + 1).map(String::as_str),
        Some("forgeclient"),
        "wrong launchTarget value in args:\n{args:#?}"
    );

    let module_path_index = args
        .iter()
        .position(|a| a == "-p")
        .expect("expected -p (module path) in args");
    let module_path = args
        .get(module_path_index + 1)
        .expect("expected module path value after -p");
    assert!(
        !module_path.contains("${"),
        "unresolved template in module path: {module_path}"
    );
    assert!(
        module_path.contains("bootstraplauncher"),
        "module path missing bootstraplauncher entry:\n{module_path}"
    );

    for var in [
        "${classpath}",
        "${natives_directory}",
        "${assets_root}",
        "${assets_index_name}",
        "${game_directory}",
        "${version_name}",
        "${library_directory}",
        "${classpath_separator}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.16.5 with `forge = "recommended"` — middle-era Forge. Uses
/// `cpw.mods.modlauncher.Launcher` (the predecessor to
/// bootstraplauncher — no JPMS module path yet) and Forge's launch
/// target here is `fmlclient`, not `forgeclient`. Same Modern
/// arguments code path as 1.20.1, different Forge-side wiring.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer"]
async fn build_command_1_16_5_forge_recommended_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.16.5")
        .forge_spec("recommended")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "cpw.mods.modlauncher.Launcher"),
        "expected modlauncher.Launcher main class in args:\n{args:#?}"
    );

    let target_index = args
        .iter()
        .position(|a| a == "--launchTarget")
        .expect("expected --launchTarget in args");
    assert_eq!(
        args.get(target_index + 1).map(String::as_str),
        Some("fmlclient"),
        "wrong launchTarget value in args:\n{args:#?}"
    );

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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.16.5 with `forge = "latest"` — same shape as the `recommended`
/// test but pins the `latest` channel, which today resolves to a
/// different build (36.2.42 vs. 36.2.34). Guards the middle-era
/// modlauncher wiring against build-specific drift.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer"]
async fn build_command_1_16_5_forge_latest_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.16.5")
        .forge_spec("latest")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "cpw.mods.modlauncher.Launcher"),
        "expected modlauncher.Launcher main class in args:\n{args:#?}"
    );

    let target_index = args
        .iter()
        .position(|a| a == "--launchTarget")
        .expect("expected --launchTarget in args");
    assert_eq!(
        args.get(target_index + 1).map(String::as_str),
        Some("fmlclient"),
        "wrong launchTarget value in args:\n{args:#?}"
    );

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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.7.10 with `forge = "recommended"` — pre-FML/Forge merge era.
/// Same Legacy-arguments + launchwrapper shape as 1.12.2 Forge, but
/// the tweak class still lives under `cpw.mods.fml` (not
/// `net.minecraftforge.fml`) — the classes weren't renamed until
/// after 1.7.10. If Forge's `minecraftArguments` merge ever drops
/// this specific tweaker, launchwrapper falls through to vanilla and
/// FML never initializes.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run; also hits Forge promotions + installer"]
async fn build_command_1_7_10_forge_recommended_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.7.10")
        .forge_spec("recommended")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.launchwrapper.Launch"),
        "expected launchwrapper main class in args:\n{args:#?}"
    );

    let tweak_index = args
        .iter()
        .position(|a| a == "--tweakClass")
        .expect("expected --tweakClass in args");
    assert_eq!(
        args.get(tweak_index + 1).map(String::as_str),
        Some("cpw.mods.fml.common.launcher.FMLTweaker"),
        "wrong tweakClass value in args:\n{args:#?}"
    );

    for var in [
        "${classpath}",
        "${natives_directory}",
        "${game_directory}",
        "${auth_session}",
        "${game_assets}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    let classpath_index = args
        .iter()
        .position(|a| a == "-cp")
        .expect("expected -cp in args");
    let classpath = args
        .get(classpath_index + 1)
        .expect("expected classpath value after -cp");
    assert!(
        classpath.contains("forge"),
        "no forge-named entry on classpath — Forge libraries missing?\n{classpath}"
    );

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.16.5 shares the `Arguments::Modern` code path with 1.20.1 but
/// pulls a different Java runtime (major 8 vs. 17) and a different
/// LWJGL library set. Catches version-specific classpath/Java-selection
/// regressions independently of the 1.20.1 test.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_16_5_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.16.5")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.client.main.Main"),
        "expected vanilla main class in args:\n{args:#?}"
    );

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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.7.10 is the last of the pre-launcher-refresh era — Legacy
/// arguments, Java 8, LWJGL 2. Exercises the same code path as 1.12.2
/// but against a materially different library set and asset index
/// vintage.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_7_10_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.7.10")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.client.main.Main"),
        "expected vanilla main class in args:\n{args:#?}"
    );

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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.6.4 is the first version to ship with the modern launcher's
/// version-JSON format, but still on Legacy arguments and the very
/// first LWJGL 2 library layout. Sits at the earliest end of the
/// current codepath's compatibility surface.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_6_4_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.6.4")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.client.main.Main"),
        "expected vanilla main class in args:\n{args:#?}"
    );

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

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// 1.2.5 is pre-1.6 — its manifest's `mainClass` is the launchwrapper
/// indirection (`net.minecraft.launchwrapper.Launch`), its args
/// reference `${auth_session}` (combined session token) instead of the
/// split `--accessToken`/`--uuid` form, and `--assetsDir` points at
/// `${game_assets}` — a name-based asset directory rather than the
/// modern hash-based `${assets_root}` layout. This test locks in that
/// all three templates get resolved by `build_command`.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_1_2_5_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("1.2.5")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.launchwrapper.Launch"),
        "expected launchwrapper main class in args:\n{args:#?}"
    );

    for var in [
        "${classpath}",
        "${natives_directory}",
        "${game_directory}",
        "${auth_session}",
        "${game_assets}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}

/// Beta 1.7 is the earliest fixture we ship and shares the pre-1.6
/// codepath with 1.2.5 (launchwrapper, `${auth_session}`,
/// `${game_assets}`, `pre-1.6` asset index). Lock in the same shape
/// checks — this is the far end of the compatibility surface.
#[tokio::test]
#[ignore = "warms the shared cart cache on first run"]
async fn build_command_b1_7_vanilla_has_expected_shape() {
    let game_dir = tempfile::tempdir().unwrap();
    let instance = Instance::builder()
        .version("b1.7")
        .build(game_dir.path().to_path_buf());

    let launcher = Launcher::new();
    let (command, _natives_directory) = launcher.build_command(&instance).await.unwrap();

    let program = command.as_std().get_program().to_string_lossy().into_owned();
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    assert!(
        program.contains("java"),
        "expected bundled java runtime, got: {program}"
    );
    assert!(
        args.iter().any(|a| a == "net.minecraft.launchwrapper.Launch"),
        "expected launchwrapper main class in args:\n{args:#?}"
    );

    for var in [
        "${classpath}",
        "${natives_directory}",
        "${game_directory}",
        "${auth_session}",
        "${game_assets}",
    ] {
        let leaked: Vec<&String> = args.iter().filter(|a| a.contains(var)).collect();
        assert!(
            leaked.is_empty(),
            "critical template {var} left unresolved in: {leaked:#?}"
        );
    }

    assert!(args.iter().any(|a| a == "-Xmx4G"), "missing -Xmx4G");
    assert!(args.iter().any(|a| a == "-Xms1G"), "missing -Xms1G");
}
