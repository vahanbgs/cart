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
}
