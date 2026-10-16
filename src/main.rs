mod board;
mod listing;
mod paths;
mod tui;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "spellbook", version = "0.2.0", about = "faeOS terminal file manager")]
struct Cli {
    /// Starting directory
    path: Option<PathBuf>,
    /// Pick a file and print (or write --output)
    #[arg(long)]
    pick: bool,
    /// Write the chosen path to this file (with --pick)
    #[arg(long)]
    output: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    let start = cli
        .path
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    if let Err(e) = tui::run(start, cli.pick, cli.output) {
        eprintln!("spellbook: {e}");
        eprintln!("  next:  run from a real terminal");
        std::process::exit(1);
    }
}
