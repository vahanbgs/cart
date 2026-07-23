mod asset;

pub use asset::AssetCache;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail};
use fs4::fs_std::FileExt;
use futures::StreamExt;
use reqwest::Client;
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;
use tokio::{fs, io::AsyncWriteExt};
use url::{Origin, Url};

use crate::Sha1Digest;

/// TTL applied to mutable JSON manifests (the top-level Mojang version list,
/// Mojang's Java-runtime index, Forge's promotions channel). Content-addressed
/// resources — anything fetched with an `expected_digest` — stay cached
/// forever; only these few "list of everything" endpoints need refresh so
/// long-lived cart installs pick up new Minecraft/Java/Forge releases.
pub const MANIFEST_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);

pub struct Cache {
    path: PathBuf,
    client: Client,
}

impl Cache {
    pub fn new(path: PathBuf, client: Client) -> Self {
        Self { path, client }
    }

    /// A path under the cache root — `{cache}/{name}`. Escape hatch for
    /// domains whose subpath is derived from a runtime value (e.g. Forge's
    /// per-flavor maven mirror path). Prefer the typed accessors
    /// (`assets_dir`, `java_dir`, `versions_dir`) where they fit.
    pub fn namespace(&self, name: impl AsRef<Path>) -> PathBuf {
        self.path.join(name)
    }

    /// Asset cache root — `{cache}/assets`.
    pub fn assets_dir(&self) -> PathBuf {
        self.namespace("assets")
    }

    /// Per-Java-component runtime directory — `{cache}/java/{component}`.
    /// `component` is typically `JavaVersionComponent::as_ref()`.
    pub fn java_dir(&self, component: impl AsRef<Path>) -> PathBuf {
        self.namespace("java").join(component)
    }

    /// Per-version JAR namespace — `{cache}/versions`. Callers compose the
    /// `<vid>/<vid>.jar` tail themselves; only NeoForge needs this today
    /// (mirroring the vanilla launcher's `versions/` layout so JPMS module
    /// naming lands on `<vid>` instead of the piston-data `client` name).
    pub fn versions_dir(&self) -> PathBuf {
        self.namespace("versions")
    }

    /// True if `url`'s target has already been persisted to the cache.
    /// Callers use this for progress reporting ("cached" vs. "download")
    /// before calling [`fetch`]; it doesn't touch the flock, so racing a
    /// concurrent writer may return `false` right before that writer's
    /// atomic-rename lands. That's fine for progress reporting.
    pub async fn is_cached(&self, url: &Url) -> anyhow::Result<bool> {
        Ok(fs::try_exists(self.path_from_url(url)?).await?)
    }

    /// Fetch `url` into the cache (or find it there) and hard-link the
    /// cache entry to `into`. Consolidates the "download once, place at
    /// one or more targets" pattern used by the Java runtime layout, the
    /// asset virtual dir, and NeoForge's versioned client alias. Parent
    /// directories of `into` are created if missing, and a pre-existing
    /// `into` with the correct source is treated as success by
    /// [`super::fs_ops::hard_link`].
    pub async fn materialize(
        &self,
        url: &Url,
        expected_digest: Option<&Sha1Digest>,
        into: &Path,
    ) -> anyhow::Result<()> {
        let source = self.fetch(url, expected_digest, None).await?;
        if let Some(parent) = into.parent() {
            fs::create_dir_all(parent).await?;
        }
        super::fs_ops::hard_link(&source, into).await
    }

    pub async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &Url,
        expected_digest: Option<&Sha1Digest>,
        max_age: Option<Duration>,
    ) -> anyhow::Result<T> {
        let path = self.fetch(url, expected_digest, max_age).await?;

        Ok(serde_json::from_str(&fs::read_to_string(path).await?)?)
    }

    /// `max_age = None` — warm cache is trusted forever (the default for
    /// content-addressed resources: client JARs, libraries, assets,
    /// SHA-1-verified manifests).
    ///
    /// `max_age = Some(ttl)` — before returning a cached entry we `stat`
    /// its mtime and re-fetch if it's older than `ttl`. On network error
    /// with a stale copy present, we log a warning and serve the stale
    /// copy (offline / Mojang-outage tolerance). This is opt-in per call
    /// site and used only for the handful of URL-mutable JSON manifests
    /// listed in [`MANIFEST_MAX_AGE`].
    pub async fn fetch(
        &self,
        url: &Url,
        expected_digest: Option<&Sha1Digest>,
        max_age: Option<Duration>,
    ) -> anyhow::Result<PathBuf> {
        let path = self.path_from_url(url)?;

        // Warm-cache fast path: if the target is already there and (for
        // TTL-tracked resources) still fresh, skip the flock entirely and
        // keep the hot path at a single `stat`. `is_stale` also does a
        // `stat`, so the fresh-cached case is two syscalls at most.
        if fs::try_exists(&path).await? && !is_stale(&path, max_age).await? {
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
        // the target with identical bytes. Under TTL, that refresh also
        // counts — no need to re-fetch if we lost the race to a peer that
        // already brought the file in-window.
        if fs::try_exists(&path).await? && !is_stale(&path, max_age).await? {
            tracing::debug!("Fetched '{url}' from the cache");
            return Ok(path);
        }

        // Everything in the network path is fallible; a `reqwest` error
        // for a TTL-tracked URL should not fail the whole run when we
        // still have a slightly-stale copy on disk. Route all downstream
        // errors through a single `Err` arm and consult the fallback
        // there.
        let network_result: anyhow::Result<NamedTempFile> = async {
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
            let temp_file = NamedTempFile::new_in(parent_directory_path)?;
            let mut file = fs::File::create(temp_file.path()).await?;
            let mut stream = response.bytes_stream();
            let mut hasher = Sha1::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                hasher.update(&chunk);
            }
            file.flush().await?;

            let computed_digest = Sha1Digest::from_bytes(hasher.finalize().into());

            if let Some(expected_digest) = expected_digest
                && *expected_digest != computed_digest
            {
                bail!("SHA-1 digest mismatch while fetching '{}'", url);
            }

            Ok(temp_file)
        }
        .await;

        let temp_file = match network_result {
            Ok(temp_file) => temp_file,
            Err(err) => {
                // TTL-tracked URLs get a stale-copy fallback so users on
                // planes / behind captive portals / during Mojang outages
                // can still launch. Untracked URLs get no fallback — a
                // failed content-addressed fetch is a real error.
                if max_age.is_some() && fs::try_exists(&path).await? {
                    tracing::warn!("failed to refresh '{url}': {err}; using stale cached copy");
                    return Ok(path);
                }
                return Err(err);
            }
        };

        tracing::debug!("Fetched '{url}' from the network");

        temp_file
            .into_temp_path()
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

/// True when `path` exists and its mtime is older than `max_age`. `None`
/// always yields `false` (the caller opts out of TTL tracking entirely).
/// A missing file yields `false` too — the surrounding `try_exists` check
/// answers "should we refetch?" for that case.
async fn is_stale(path: &Path, max_age: Option<Duration>) -> anyhow::Result<bool> {
    let Some(max_age) = max_age else {
        return Ok(false);
    };
    let metadata = match fs::metadata(path).await {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let mtime = metadata.modified()?;
    // A negative `duration_since` (clock skew, or someone `touch -t`'d
    // the file into the future) reports the entry as fresh — better than
    // refetching in a loop when we can't trust the numbers.
    let age = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO);
    Ok(age > max_age)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use reqwest::Client;
    use tempfile::TempDir;
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

    /// Seed a cache with `contents` at the path where `url` would be
    /// stored, and back-date its mtime by `age`. Returns the on-disk path.
    /// Wraps the boilerplate the TTL tests share.
    fn seed_cache_entry(cache: &Cache, url: &Url, contents: &[u8], age: Duration) -> PathBuf {
        let path = cache.path_from_url(url).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
        path
    }

    /// Unroutable target. Loopback port 1 is virtually never listening,
    /// so the OS returns ECONNREFUSED immediately — no wall-clock wait.
    fn unroutable_url(path: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:1/{path}")).unwrap()
    }

    /// A fresh cached entry (mtime within `max_age`) skips the network
    /// entirely. We prove that by pointing at an unroutable URL: if the
    /// warm-cache fast path weren't gated by mtime, this would fail
    /// with ECONNREFUSED.
    #[tokio::test]
    async fn fresh_entry_within_ttl_serves_from_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = Cache::new(tmp.path().to_owned(), Client::new());
        let url = unroutable_url("manifest.json");
        seed_cache_entry(&cache, &url, b"{\"cached\":true}", Duration::from_secs(60));

        let path = cache
            .fetch(&url, None, Some(Duration::from_secs(6 * 3600)))
            .await
            .expect("fresh cache entry should be returned without a network hit");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"cached\":true}");
    }

    /// A stale entry (mtime older than `max_age`) forces the download
    /// path. When that fails and the stale copy is still on disk, we
    /// serve the stale bytes rather than propagating the network error —
    /// users on planes / behind captive portals / during Mojang outages
    /// still get to launch.
    #[tokio::test]
    async fn stale_entry_falls_back_on_network_error() {
        let tmp = TempDir::new().unwrap();
        let cache = Cache::new(tmp.path().to_owned(), Client::new());
        let url = unroutable_url("manifest.json");
        seed_cache_entry(
            &cache,
            &url,
            b"{\"stale\":true}",
            Duration::from_secs(24 * 3600),
        );

        let path = cache
            .fetch(&url, None, Some(Duration::from_secs(6 * 3600)))
            .await
            .expect("stale copy should be returned when the network fetch fails");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"stale\":true}");
    }

    /// Without TTL opt-in, an unroutable URL surfaces the network error
    /// even when a cached copy is missing. Guards against a regression
    /// where the fallback branch swallows errors for non-TTL callers.
    #[tokio::test]
    async fn no_ttl_and_no_cache_returns_network_error() {
        let tmp = TempDir::new().unwrap();
        let cache = Cache::new(tmp.path().to_owned(), Client::new());
        let url = unroutable_url("nonexistent.json");

        let err = cache.fetch(&url, None, None).await.unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("connect")
                || err.to_string().to_ascii_lowercase().contains("refused")
                || err.to_string().to_ascii_lowercase().contains("error"),
            "expected a network error, got: {err}"
        );
    }
}
