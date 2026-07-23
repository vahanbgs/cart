//! Race-safe filesystem primitives for the launcher pipeline.
//!
//! The `link(2)` and `symlink(2)` syscalls are atomic, but the classic
//! `if !try_exists(target) { link(...) }` idiom is not — two concurrent
//! callers both pass the existence check and one gets `EEXIST` from the
//! syscall. Since our callers always link to a deterministic target from
//! deterministic source content (Java runtime files, asset objects,
//! version-scoped client JARs), `AlreadyExists` here means another caller
//! already produced the identical entry; treat it as success.

use std::{io, path::Path};

use anyhow::Context;
use tokio::fs;

/// Create a hard link at `target` pointing at `source`, treating an
/// existing `target` as success.
pub async fn hard_link(source: impl AsRef<Path>, target: impl AsRef<Path>) -> anyhow::Result<()> {
    let source = source.as_ref();
    let target = target.as_ref();
    match fs::hard_link(source, target).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("hard-link {} → {}", source.display(), target.display())),
    }
}

/// Create a symlink at `link_path` pointing at `original` (which may be a
/// relative path string interpreted by the OS at resolution time), treating
/// an existing `link_path` as success.
pub async fn symlink(
    original: impl AsRef<Path>,
    link_path: impl AsRef<Path>,
) -> anyhow::Result<()> {
    let original = original.as_ref();
    let link_path = link_path.as_ref();
    match fs::symlink(original, link_path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err)
            .with_context(|| format!("symlink {} → {}", link_path.display(), original.display())),
    }
}
