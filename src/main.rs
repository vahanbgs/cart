use cart::{Config, Instance};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::load().await?;

    let instance = Instance::builder()
        .version(config.minecraft_version())
        .build(config.manifest_directory().join("minecraft/"));

    instance.launch().await?;

    Ok(())
}
