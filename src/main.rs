use cart::{Cli, Instance, Manifest};
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let manifest_path = Manifest::resolve_path(&cli).await?;
    let manifest_directory = manifest_path.parent().unwrap();
    let manifest = Manifest::load(&cli, &manifest_path).await?;

    let instance = Instance::builder()
        .version(manifest.minecraft_version())
        .build(manifest_directory.join("minecraft/"));

    instance.launch().await?;

    Ok(())
}
