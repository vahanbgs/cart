use std::path::{Path, PathBuf};

use super::{Launcher, Loader};

pub struct Instance {
    directory: PathBuf,
    version: String,
    loader: Option<Loader>,
}

impl Instance {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self::builder().build(directory.into())
    }

    pub fn builder() -> InstanceBuilder {
        InstanceBuilder {
            version: "latest".to_string(),
            loader: None,
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Mod loader for this instance. `None` means vanilla.
    pub fn loader(&self) -> Option<&Loader> {
        self.loader.as_ref()
    }

    pub async fn launch(&self) -> anyhow::Result<()> {
        Launcher::new().launch(self).await
    }
}

pub struct InstanceBuilder {
    version: String,
    loader: Option<Loader>,
}

impl InstanceBuilder {
    pub fn build(self, directory: impl Into<PathBuf>) -> Instance {
        Instance {
            directory: directory.into(),
            version: self.version,
            loader: self.loader,
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn loader(mut self, loader: Loader) -> Self {
        self.loader = Some(loader);
        self
    }
}
