//! Shared helpers for integration tests. Cargo treats `tests/common/mod.rs`
//! as a submodule (via `mod common;` in each test file) rather than as its
//! own test binary.

#![allow(dead_code)]

use std::{path::PathBuf, sync::LazyLock};

use tempfile::TempDir;

/// A cache directory scoped to the current test binary. First call in the
/// process creates a fresh `TempDir`; subsequent calls in the same binary
/// reuse it, so cold-cache downloads happen once per `cargo test` run
/// rather than once per test function. The `TempDir` is dropped when the
/// process exits.
///
/// Rooted under [`CARGO_TARGET_TMPDIR`] (cargo's per-package integration
/// test temp dir, typically `target/tmp/`) rather than the system
/// `TMPDIR`. This matters because the launcher suites pull in
/// multi-GB Java runtimes and MC assets, and the system `/tmp` is often a
/// small RAM-backed tmpfs that fills up mid-run. `CARGO_TARGET_TMPDIR`
/// lives on the same disk as the source tree, which is always spacious
/// enough.
///
/// Using this in place of `Launcher::new()` isolates tests from the
/// developer's real `~/.cache/cart/` — behavior no longer depends on
/// whatever the user has previously launched with cart.
///
/// [`CARGO_TARGET_TMPDIR`]: https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-crates
pub fn cache_dir() -> PathBuf {
    static DIR: LazyLock<TempDir> = LazyLock::new(|| {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(&base).expect("create CARGO_TARGET_TMPDIR");
        TempDir::new_in(&base).expect("create per-test-binary cache dir")
    });
    DIR.path().to_owned()
}
