use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "akagi", about = "Akagi - Mahjong AI Assistant")]
pub struct Cli {
    #[arg(short, long, help = "Path to config.toml")]
    pub config: Option<PathBuf>,
}
