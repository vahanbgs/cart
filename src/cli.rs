pub mod args;

mod build;
mod curseforge;
mod disable;
mod enable;
mod export;
mod hit_view;
mod init;
mod list;
mod modrinth;
mod remove;
mod run;
mod update;

pub use args::*;

impl Cli {
    /// Explicit `(project_dir, manifest_path)` pair when `-C` was passed.
    /// `None` means walk up from cwd looking for `cart.toml`.
    pub fn manifest_path(&self) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        self.directory
            .as_ref()
            .map(|dir| (dir.clone(), dir.join("cart.toml")))
    }
}
