use clap::Parser;

#[derive(Parser)]
pub struct Cli {
    #[arg(long = "mv")]
    pub minecraft_version: Option<String>,
}
