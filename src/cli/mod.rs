mod chat;
mod dashboard;
mod history;
mod image;
mod list_tui;
mod self_cmd;
mod task;
mod version;
mod video;

use anyhow::Result;
use clap::Subcommand;

pub use chat::ChatArgs;
pub use image::ImageArgs;
pub use video::VideoArgs;

pub fn print_version_short() {
    version::print_short();
}

#[derive(Subcommand)]
pub enum Command {
    /// Generate images via Agnes Image API
    Image(ImageArgs),
    /// Generate videos via Agnes Video API
    Video(VideoArgs),
    /// List/show/wait video async tasks
    Task(task::TaskCmd),
    /// List/show generation history (SQLite)
    History(history::HistoryCmd),
    /// List/show assets in the resource library (asset://)
    Asset(history::AssetCmd),
    /// Manage configuration
    Config(crate::config::ConfigCmd),
    /// Launch terminal dashboard (ratatui)
    Dashboard,
    /// Chat with Agnes agent
    Chat(ChatArgs),
    /// Show version information
    Version(version::VersionCmd),
    /// Manage the installed binary (update / uninstall)
    #[command(name = "self")]
    ManageSelf(self_cmd::SelfCmd),
}

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Image(args) => image::run(args),
        Command::Video(args) => video::run(args),
        Command::Task(cmd) => task::run(cmd),
        Command::History(cmd) => history::run_history(cmd),
        Command::Asset(cmd) => history::run_asset(cmd),
        Command::Config(cmd) => crate::config::run(cmd),
        Command::Dashboard => dashboard::run(),
        Command::Chat(args) => chat::run(args),
        Command::Version(cmd) => version::run(cmd),
        Command::ManageSelf(cmd) => self_cmd::run(cmd),
    }
}
