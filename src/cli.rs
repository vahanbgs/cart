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
mod remove;
mod run;
mod scaffold;
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
