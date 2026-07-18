mod asset;

pub use asset::AssetCache;

use std::path::{Path, PathBuf};

use anyhow::bail;
use futures::StreamExt;
use reqwest::Client;
use sha1::{Digest, Sha1};
use tokio::{fs, io::AsyncWriteExt};
use url::{Origin, Url};

use crate::Sha1Digest;

pub struct Cache<'a> {
    path: PathBuf,
    client: &'a Client,
}

impl<'a> Cache<'a> {
    pub fn new(path: PathBuf, client: &'a Client) -> Self {
        Self { path, client }
    }

    pub fn directory(&self) -> &Path {
        &self.path
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

        if let Some(expected_digest) = expected_digest {
            if *expected_digest != computed_digest {
                bail!("SHA-1 digest mismatch while fetching '{}'", url);
            }
        }

        Ok(path)
    }

    fn path_from_url(&self, url: &Url) -> anyhow::Result<PathBuf> {
        let Origin::Tuple(_, host, _) = url.origin() else {
            bail!("Could not extract host from URL: {}", url);
        };

        let Some(url_path) = url.path().strip_prefix('/') else {
            bail!("Could not remove initial '/' from URL path: {}", url);
        };

        Ok(self.path.join(host.to_string()).join(url_path))
    }
}
