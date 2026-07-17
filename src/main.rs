mod launcher;

use launcher::Launcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let launcher = Launcher::new();

    launcher.launch().await
}
