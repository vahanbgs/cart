use cart::{CartManifest, Cli, Instance};
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let manifest_path = CartManifest::resolve_path(&cli).await?;
    let manifest_directory = manifest_path.parent().unwrap();
    let cart_manifest = CartManifest::load(&cli, &manifest_path).await?;

    let instance = Instance::builder()
        .version(cart_manifest.minecraft_version())
        .build(manifest_directory.join("minecraft/"));

    instance.launch().await?;

    Ok(())
}
