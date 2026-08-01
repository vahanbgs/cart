//! Tiny content-addressed cache for mod icons.
//!
//! Sits alongside `launcher::Cache` rather than reusing it: icons don't
//! need SHA-1 verification against a known digest, atomic renames on
//! integrity failure, per-file flocking, or the URL-mirroring layout
//! (which would sprinkle `avatars/…/…/*.png` under
//! `~/.cache/cart/media.forgecdn.net/`). We keep them in a flat,
//! shard-by-first-two-hex-chars tree keyed by the SHA-1 of the source URL.
//!
//! Icons that fail to fetch degrade gracefully at the call site — a
//! transient CDN blip should never take down `cart search`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use directories_next::ProjectDirs;
use reqwest::Client;
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::fs;
use url::Url;

/// Cap per-icon fetch time so the search-view doesn't stall on a slow
/// CDN. Icons are ~10-30 KB in practice, so 5s is plenty of headroom.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

pub struct IconCache {
    root: PathBuf,
    http: Client,
}

impl IconCache {
    pub fn new(root: PathBuf, http: Client) -> Self {
        Self { root, http }
    }

    /// Convenience constructor pointing at `~/.cache/cart/icons/` (or the
    /// platform equivalent). Kept parallel to `Launcher::new` so callers
    /// don't have to duplicate the `ProjectDirs` lookup.
    pub fn shared() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "cart")
            .context("could not locate a valid home directory for the icon cache")?;
        let root = dirs.cache_dir().join("icons");
        Ok(Self::new(root, Client::new()))
    }

    /// Path where `url` would live in the cache. Pure function — does no I/O.
    pub fn path_for(&self, url: &Url) -> PathBuf {
        let key = hex::encode(Sha1::digest(url.as_str().as_bytes()));
        self.root.join(&key[..2]).join(&key[2..])
    }

    /// Return the on-disk path for `url`, downloading it if absent.
    /// A concurrent fetch by another process is safe: the final rename
    /// is atomic on the same filesystem, and repeat downloads just
    /// overwrite each other with byte-identical content.
    pub async fn get(&self, url: &Url) -> Result<PathBuf> {
        let path = self.path_for(url);
        if fs::try_exists(&path).await? {
            return Ok(path);
        }
        let parent = path.parent().expect("path_for always shards under root");
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create icon cache dir {}", parent.display()))?;

        let bytes = self
            .http
            .get(url.clone())
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("GET {url}"))?
            .bytes()
            .await
            .with_context(|| format!("read body of {url}"))?;

        // Temp-file-and-rename so a killed process doesn't leave a
        // half-written file on disk. `persist` is atomic within a
        // filesystem, which is what we have (parent is under `root`).
        let tmp = NamedTempFile::new_in(parent)
            .with_context(|| format!("create temp file in {}", parent.display()))?;
        fs::write(tmp.path(), &bytes)
            .await
            .with_context(|| format!("write icon body to {}", tmp.path().display()))?;
        tmp.persist(&path)
            .with_context(|| format!("persist icon to {}", path.display()))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn path_for_shards_by_sha1_prefix() {
        let root = TempDir::new().unwrap();
        let cache = IconCache::new(root.path().to_owned(), Client::new());
        let a = cache.path_for(&Url::parse("https://cdn.example/a.png").unwrap());
        let b = cache.path_for(&Url::parse("https://cdn.example/b.png").unwrap());

        // Sharded: first two hex chars of the SHA-1 are the parent dir.
        assert_eq!(a.parent().unwrap().parent().unwrap(), root.path());
        assert_eq!(
            a.parent().unwrap().file_name().unwrap().to_str().unwrap().len(),
            2
        );
        // Same URL → same path.
        assert_eq!(
            a,
            cache.path_for(&Url::parse("https://cdn.example/a.png").unwrap())
        );
        // Distinct URL → distinct path.
        assert_ne!(a, b);
    }
}
