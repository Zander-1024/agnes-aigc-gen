use anyhow::Result;
use clap::{Args, Subcommand};

use crate::install::{
    DEFAULT_REPO, InstallState, PKG_VERSION, detect_platform, fetch_latest_version, is_upgrade_available, platform_slug,
};

#[derive(Args)]
pub struct VersionCmd {
    #[command(subcommand)]
    pub action: Option<VersionAction>,

    /// Show binary path, platform, and recorded skill targets
    #[arg(long)]
    pub long: bool,
}

#[derive(Subcommand)]
pub enum VersionAction {
    /// Compare with the latest GitHub release
    Check,
    /// Print embedded CHANGELOG
    Changelog,
}

pub fn print_short() {
    println!("agnes-aigc-gen {PKG_VERSION}");
}

pub fn run(cmd: VersionCmd) -> Result<()> {
    match cmd.action {
        None => print_version(cmd.long)?,
        Some(VersionAction::Check) => run_check()?,
        Some(VersionAction::Changelog) => print_changelog(),
    }
    Ok(())
}

fn print_version(long: bool) -> Result<()> {
    print_short();
    if !long {
        return Ok(());
    }
    let platform = detect_platform()?;
    println!("platform: {}", platform_slug(&platform));
    if let Ok(exe) = std::env::current_exe() {
        println!("binary: {}", exe.display());
    }
    if let Ok(state) = InstallState::load_or_discover() {
        println!("installed_version: {}", state.installed_version);
        for target in &state.skill_targets {
            println!("skill_target: {}", target.parent_dir);
        }
    }
    Ok(())
}

fn run_check() -> Result<()> {
    let latest = fetch_latest_version(DEFAULT_REPO)?;
    let current = PKG_VERSION;
    println!("current: v{current}");
    println!("latest:  v{latest}");
    if is_upgrade_available(&latest, current) {
        println!("Update available. Run: agnes-aigc-gen self update");
    } else {
        println!("You are up to date.");
    }
    Ok(())
}

fn print_changelog() {
    println!("{}", include_str!("../../CHANGELOG.md"));
}
