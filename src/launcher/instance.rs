use std::path::{Path, PathBuf};

use super::Launcher;

pub struct Instance {
    directory: PathBuf,
    version: String,
    forge_spec: Option<String>,
}

impl Instance {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self::builder().build(directory.into())
    }

    pub fn builder() -> InstanceBuilder {
        InstanceBuilder {
            version: "latest".to_string(),
            forge_spec: None,
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// The raw Forge version spec from `cart.toml`: `"latest"`, `"recommended"`,
    /// or a specific version number like `"47.3.12"`.  `None` means vanilla.
    pub fn forge_spec(&self) -> Option<&str> {
        self.forge_spec.as_deref()
    }

    pub async fn launch(&self) -> anyhow::Result<()> {
        Launcher::new().launch(self).await
    }
}

pub struct InstanceBuilder {
    version: String,
    forge_spec: Option<String>,
}

impl InstanceBuilder {
    pub fn build(self, directory: impl Into<PathBuf>) -> Instance {
        Instance {
            directory: directory.into(),
            version: self.version,
            forge_spec: self.forge_spec,
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn forge_spec(mut self, spec: impl Into<String>) -> Self {
        self.forge_spec = Some(spec.into());
        self
    }
}
