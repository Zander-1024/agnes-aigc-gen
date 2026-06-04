mod agent;
mod api;
mod cli;
mod config;
mod crypto;
mod db;
mod logging;
mod media;
mod output;
mod ratio;
mod ui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "agnes-aigc-gen", about = "Agnes AI image & video generation CLI")]
struct Cli {
    /// Print detailed logs to stderr for troubleshooting
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: cli::Command,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.verbose);
    cli::run(cli.command)
}
