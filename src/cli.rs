pub mod args;

mod build;
mod curseforge;
mod deps;
mod disable;
mod enable;
mod export;
mod hit_view;
mod icon_cache;
mod init;
mod list;
mod modrinth;
mod new;
mod picker;
mod remove;
mod run;
mod scaffold;
mod update;

pub use args::*;

impl Cli {
    /// Explicit `(project_dir, manifest_path)` pair when `-C` and/or
    /// `--manifest-path` were passed. `None` means walk up from cwd
    /// looking for `cart.toml`.
    ///
    /// `--manifest-path` mirrors cargo: filename must be `cart.toml`; a
    /// relative path is resolved against `-C` if given, else cwd; an
    /// absolute path stands alone.
    pub fn resolve_manifest_paths(
        &self,
    ) -> anyhow::Result<Option<(std::path::PathBuf, std::path::PathBuf)>> {
        match (&self.manifest_path, &self.directory) {
            (Some(file), base) => {
                if file.file_name() != Some(std::ffi::OsStr::new("cart.toml")) {
                    anyhow::bail!(
                        "--manifest-path must point at a file named `cart.toml`, got {}",
                        file.display()
                    );
                }
                let resolved = match base {
                    Some(dir) if file.is_relative() => dir.join(file),
                    _ => file.clone(),
                };
                let parent = resolved
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                Ok(Some((parent, resolved)))
            }
            (None, Some(dir)) => Ok(Some((dir.clone(), dir.join("cart.toml")))),
            (None, None) => Ok(None),
        }
    }
}
