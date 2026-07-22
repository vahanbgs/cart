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
/// Using this in place of `Launcher::new()` isolates tests from the
/// developer's real `~/.cache/cart/` — behavior no longer depends on
/// whatever the user has previously launched with cart.
pub fn cache_dir() -> PathBuf {
    static DIR: LazyLock<TempDir> =
        LazyLock::new(|| TempDir::new().expect("create per-test-binary cache dir"));
    DIR.path().to_owned()
}
