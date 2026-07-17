use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::bail;
use cart::{CartManifest, Cli, Instance};
use clap::Parser;
use tokio::fs;

async fn try_find_manifest_file() -> anyhow::Result<PathBuf> {
    let mut current_directory = env::current_dir()?;

    loop {
        let manifest_path = current_directory.join("cart.toml");

        if fs::try_exists(&manifest_path).await? {
            return Ok(manifest_path);
        }

        if !current_directory.pop() {
            break;
        }
    }

    bail!("Could not find cart.toml manifest file");
}

async fn resolve_manifest_path(cli: &Cli) -> anyhow::Result<PathBuf> {
    if let Some(manifest_path) = &cli.manifest {
        return Ok(manifest_path.to_owned());
    }

    try_find_manifest_file().await
}

async fn load_manifest_file(path: &Path) -> anyhow::Result<CartManifest> {
    let manifest = toml::from_str(&fs::read_to_string(path).await?)?;

    Ok(manifest)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let manifest_path = resolve_manifest_path(&cli).await?;
    let mut cart_manifest = load_manifest_file(&manifest_path).await?;
    cart_manifest.override_with(&cli);
    let manifest_directory = manifest_path.parent().unwrap();

    let instance = Instance::builder()
        .version(cart_manifest.minecraft_version())
        .build(manifest_directory.join("minecraft/"));

    instance.launch().await?;

    Ok(())
}
