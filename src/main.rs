mod agent;
mod api;
mod cli;
mod config;
mod crypto;
mod db;
mod install;
mod logging;
mod media;
mod output;
mod ratio;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser)]
#[command(name = "agnes-aigc-gen", about = "Agnes AI image & video generation CLI")]
struct Cli {
    /// Print current version and exit
    #[arg(short = 'v', visible_short_aliases = ['V'], global = true)]
    show_version: bool,

    /// Print detailed logs to stderr for troubleshooting
    #[arg(long = "verbose", global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.show_version {
        cli::print_version_short();
        return Ok(());
    }
    let command = cli.command.context("subcommand required")?;
    logging::init(cli.verbose);
    cli::run(command)
}
