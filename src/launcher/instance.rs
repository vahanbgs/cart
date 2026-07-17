use std::path::{Path, PathBuf};

pub struct Instance {
    directory: PathBuf,
    version: String,
}

impl Instance {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self::builder().build(directory.into())
    }

    pub fn builder() -> InstanceBuilder {
        InstanceBuilder {
            version: "latest".to_string(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub async fn launch(&self) -> anyhow::Result<()> {
        super::Launcher::new().launch(self).await
    }
}

pub struct InstanceBuilder {
    version: String,
}

impl InstanceBuilder {
    pub fn build(self, directory: impl Into<PathBuf>) -> Instance {
        let directory = directory.into();
        let version = self.version;

        Instance { directory, version }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();

        self
    }
}
