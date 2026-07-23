mod asset;

pub use asset::AssetCache;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use fs4::fs_std::FileExt;
use futures::StreamExt;
use reqwest::Client;
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::{fs, io::AsyncWriteExt};
use url::{Origin, Url};

use crate::Sha1Digest;

pub struct Cache {
    path: PathBuf,
    client: Client,
}

impl Cache {
    pub fn new(path: PathBuf, client: Client) -> Self {
        Self { path, client }
    }

    pub fn directory(&self) -> &Path {
        &self.path
    }

    /// True if `url`'s target has already been persisted to the cache.
    /// Callers use this for progress reporting ("cached" vs. "download")
    /// before calling [`fetch`]; it doesn't touch the flock, so racing a
    /// concurrent writer may return `false` right before that writer's
    /// atomic-rename lands. That's fine for progress reporting.
    pub async fn is_cached(&self, url: &Url) -> anyhow::Result<bool> {
        Ok(fs::try_exists(self.path_from_url(url)?).await?)
    }

    pub async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &Url,
        expected_digest: Option<&Sha1Digest>,
    ) -> anyhow::Result<T> {
        let path = self.fetch(url, expected_digest).await?;

        Ok(serde_json::from_str(&fs::read_to_string(path).await?)?)
    }

    pub async fn fetch(
        &self,
        url: &Url,
        expected_digest: Option<&Sha1Digest>,
    ) -> anyhow::Result<PathBuf> {
        let path = self.path_from_url(url)?;

        // Warm-cache fast path: if the target is already there, skip the
        // flock entirely and keep the hot path at a single `stat`.
        if fs::try_exists(&path).await? {
            tracing::debug!("Fetched '{url}' from the cache");
            return Ok(path);
        }

        let Some(parent_directory_path) = path.parent() else {
            bail!("Path '{:?}' has no parent directory.", path);
        };

        // Cross-process + cross-instance advisory lock on the target file.
        // `flock(2)` is per open file description on Linux, so two threads
        // in this process opening the lockfile independently still contend
        // correctly — one primitive handles both intra-process races
        // (multiple `Launcher`s in the test suite sharing the per-binary
        // temp cache) and inter-process races (two `cart` invocations
        // against `~/.cache/cart/`). Mirrors the pattern in
        // [`super::forge::install`]. The lockfile sits adjacent to the
        // target with a `.NAME.lock` name so `ls` doesn't surface it, and
        // we never unlink it — a delete+relock race would be worse than
        // leaving a zero-byte file behind.
        fs::create_dir_all(parent_directory_path).await?;
        let lock_path = parent_directory_path.join(format!(
            ".{}.lock",
            path.file_name()
                .expect("path_from_url always yields a filename")
                .to_string_lossy()
        ));
        let _fetch_guard =
            tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .truncate(false)
                    .open(&lock_path)?;
                file.lock_exclusive()?;
                Ok(file)
            })
            .await
            .context("await cache fetch lock task")?
            .context("acquire cache fetch lock")?;

        // Re-check under the lock: another writer may have completed the
        // download while we blocked, and redoing the work would just churn
        // the target with identical bytes.
        if fs::try_exists(&path).await? {
            tracing::debug!("Fetched '{url}' from the cache");
            return Ok(path);
        }

        let response = self
            .client
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?;

        // Stream into a uniquely-named sibling of `path` and atomically
        // rename it into place once the digest verifies. The flock makes
        // this a single-writer critical section, so the temp file's real
        // job here is crash safety: readers never see a half-written
        // target, and any early exit drops the `TempPath` which unlinks
        // the partial file — no `.tmp` litter left behind.
        let temp_path = NamedTempFile::new_in(parent_directory_path)?.into_temp_path();
        let mut file = fs::File::create(&temp_path).await?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha1::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
        }
        file.flush().await?;

        tracing::debug!("Fetched '{url}' from the network");

        let computed_digest = Sha1Digest::from_bytes(hasher.finalize().into());

        if let Some(expected_digest) = expected_digest
            && *expected_digest != computed_digest
        {
            bail!("SHA-1 digest mismatch while fetching '{}'", url);
        }

        temp_path
            .persist(&path)
            .with_context(|| format!("persist cached download for '{url}'"))?;

        Ok(path)
    }

    pub(crate) fn path_from_url(&self, url: &Url) -> anyhow::Result<PathBuf> {
        let Origin::Tuple(_, host, _) = url.origin() else {
            bail!("Could not extract host from URL: {}", url);
        };

        let Some(url_path) = url.path().strip_prefix('/') else {
            bail!("Could not remove initial '/' from URL path: {}", url);
        };

        Ok(self.path.join(host.to_string()).join(url_path))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use reqwest::Client;
    use url::Url;

    use super::Cache;

    fn cache() -> Cache {
        Cache::new(PathBuf::from("/cache"), Client::new())
    }

    #[test]
    fn path_from_url_maps_host_and_path() {
        let url = Url::parse("https://piston-data.mojang.com/v1/objects/abc/client.jar").unwrap();
        let path = cache().path_from_url(&url).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/cache/piston-data.mojang.com/v1/objects/abc/client.jar"),
        );
    }

    /// `url.path()` strips the query string, so two URLs differing only in
    /// query would collide in the cache. Piston/Modrinth CDNs don't use
    /// query strings for content addresses today; if that ever changes,
    /// this test forces the discussion.
    #[test]
    fn path_from_url_ignores_query_string() {
        let bare = Url::parse("https://example.com/a/b.jar").unwrap();
        let with_query = Url::parse("https://example.com/a/b.jar?token=xyz").unwrap();
        assert_eq!(
            cache().path_from_url(&bare).unwrap(),
            cache().path_from_url(&with_query).unwrap(),
        );
    }

    /// Opaque URLs (`data:`, `file:` on some platforms) have no host tuple.
    /// We rely on this to reject nonsense inputs before creating a cache
    /// directory named after the empty string.
    #[test]
    fn path_from_url_rejects_opaque_origin() {
        let url = Url::parse("data:text/plain,hello").unwrap();
        assert!(cache().path_from_url(&url).is_err());
    }
}
