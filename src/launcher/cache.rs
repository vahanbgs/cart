mod asset;
mod mod_cache;

pub use asset::AssetCache;
pub use mod_cache::ModCache;

use std::path::{Path, PathBuf};

use anyhow::bail;
use futures::StreamExt;
use reqwest::Client;
use sha1::{Digest, Sha1};
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

    /// The shared HTTP client. Exposed so callers can make uncached GETs
    /// (e.g. Fabric's loader listing collides with the parametric profile
    /// URL under the URL-mirrored cache layout and shouldn't be cached
    /// anyway — new loaders release regularly and users would get stale
    /// answers).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
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

        let Some(parent_directory_path) = path.parent() else {
            bail!("Path '{:?}' has no parent directory.", path);
        };

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

        fs::create_dir_all(parent_directory_path).await?;

        let mut file = fs::File::create(&path).await?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha1::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
        }

        tracing::debug!("Fetched '{url}' from the network");

        let computed_digest = Sha1Digest::from_bytes(hasher.finalize().into());

        if let Some(expected_digest) = expected_digest
            && *expected_digest != computed_digest
        {
            bail!("SHA-1 digest mismatch while fetching '{}'", url);
        }

        Ok(path)
    }

    pub fn path_from_url(&self, url: &Url) -> anyhow::Result<PathBuf> {
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
