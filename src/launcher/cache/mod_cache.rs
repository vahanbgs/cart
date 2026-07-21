use std::path::PathBuf;

use url::Url;

use super::Cache;

pub struct ModCache<'cache> {
    cache: &'cache Cache,
}

impl<'cache> ModCache<'cache> {
    pub fn new(cache: &'cache Cache) -> Self {
        Self { cache }
    }

    pub async fn fetch_mod(&self, url: &Url) -> anyhow::Result<PathBuf> {
        self.cache.fetch(url, None).await
    }

    /// Local path a mod at `url` would occupy in the cache — used to check
    /// cache presence before fetching so callers can report cache hits vs.
    /// network downloads.
    pub fn path_from_url(&self, url: &Url) -> anyhow::Result<PathBuf> {
        self.cache.path_from_url(url)
    }
}
