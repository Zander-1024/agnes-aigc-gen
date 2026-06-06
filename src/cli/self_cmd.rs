use anyhow::Result;
use clap::{Args, Subcommand};

use crate::install::{UninstallOptions, UpdateOptions, run_uninstall, run_update};

#[derive(Args)]
pub struct SelfCmd {
    #[command(subcommand)]
    pub action: SelfAction,
}

#[derive(Subcommand)]
pub enum SelfAction {
    /// Check GitHub for a newer release and update the installed binary
    Update {
        /// Skip prompts and update the binary only
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Update skill files to previously recorded targets without prompting
        #[arg(long = "update-skill")]
        update_skill: bool,
        /// Do not offer or perform skill updates
        #[arg(long = "no-skill")]
        no_skill: bool,
        /// Comma-separated skill parent directories
        #[arg(long = "skill-dirs")]
        skill_dirs: Option<String>,
        /// GitHub repo slug (owner/name)
        #[arg(long = "repo")]
        repo: Option<String>,
    },
    /// Remove the installed binary, skills, and optionally local data
    Uninstall {
        /// Non-interactive mode with conservative defaults
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Remove recorded skill directories without prompting
        #[arg(long = "remove-skills")]
        remove_skills: bool,
        /// Remove config, database, and chat sessions without prompting
        #[arg(long = "remove-data")]
        remove_data: bool,
    },
}

pub fn run(cmd: SelfCmd) -> Result<()> {
    match cmd.action {
        SelfAction::Update { yes, update_skill, no_skill, skill_dirs, repo } => {
            run_update(UpdateOptions { yes, update_skill, no_skill, skill_dirs, repo })
        }
        SelfAction::Uninstall { yes, remove_skills, remove_data } => run_uninstall(UninstallOptions {
            yes,
            remove_binary: if yes { Some(true) } else { None },
            remove_skills: if remove_skills { Some(true) } else { None },
            remove_data: if remove_data { Some(true) } else { None },
        }),
    }
}
