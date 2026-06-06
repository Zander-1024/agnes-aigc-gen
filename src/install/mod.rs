mod binary;
mod platform;
mod prompt;
mod release;
mod skill;
pub mod state;

pub use binary::replace_binary;
pub use platform::{detect_platform, platform_slug};
pub use prompt::{SkillUpdateChoice, prompt_skill_update};
pub use release::{DEFAULT_REPO, fetch_latest_version, is_upgrade_available, release_tag};
pub use skill::{default_skill_parent, install_skill_from_release};
pub use state::InstallState;

use anyhow::Result;

pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run_update(options: UpdateOptions) -> Result<()> {
    let repo = options.repo.as_deref().unwrap_or(DEFAULT_REPO);
    let mut install_state = InstallState::load_or_discover()?;
    let latest = fetch_latest_version(repo)?;
    let current = PKG_VERSION;

    if !is_upgrade_available(&latest, current) {
        println!("agnes-aigc-gen {current} is already up to date (latest: v{latest}).");
        return Ok(());
    }

    println!("Upgrading agnes-aigc-gen v{current} -> v{latest} ...");
    let platform = detect_platform()?;
    let tag = release_tag(&latest);
    let archive_bytes = release::download_release_archive(repo, &tag, &platform)?;
    release::verify_checksum(repo, &tag, &platform, &archive_bytes)?;
    let new_binary = binary::extract_binary(&archive_bytes, &platform)?;
    replace_binary(&install_state.binary_path, &new_binary)?;

    install_state.installed_version = latest.clone();
    let skill_updated = match resolve_skill_choice(&options, &install_state)? {
        Some(parents) => {
            install_skill_from_release(repo, &tag, &parents)?;
            install_state.skill_targets = parents
                .iter()
                .map(|parent| state::SkillTarget { parent_dir: parent.clone() })
                .collect();
            true
        }
        None => false,
    };

    install_state.save()?;
    println!("Updated to v{latest}.");
    if !skill_updated && !options.no_skill {
        println!("Skill files were not updated. Run `agnes-aigc-gen self update --update-skill` to refresh.");
    }
    Ok(())
}

fn resolve_skill_choice(options: &UpdateOptions, state: &InstallState) -> Result<Option<Vec<String>>> {
    if options.no_skill {
        return Ok(None);
    }
    if let Some(dirs) = &options.skill_dirs {
        return Ok(Some(parse_skill_dirs(dirs)));
    }
    if options.update_skill {
        let parents = state.recommended_skill_parents();
        return Ok(Some(parents));
    }
    if options.yes || !prompt::is_tty() {
        return Ok(None);
    }
    match prompt_skill_update(state)? {
        SkillUpdateChoice::Previous => Ok(Some(state.recommended_skill_parents())),
        SkillUpdateChoice::DefaultOnly => Ok(Some(vec![default_skill_parent()?])),
        SkillUpdateChoice::Custom(paths) => Ok(Some(paths)),
        SkillUpdateChoice::Skip => Ok(None),
    }
}

fn parse_skill_dirs(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub struct UpdateOptions {
    pub yes: bool,
    pub update_skill: bool,
    pub no_skill: bool,
    pub skill_dirs: Option<String>,
    pub repo: Option<String>,
}

pub fn run_uninstall(options: UninstallOptions) -> Result<()> {
    let state = InstallState::load_or_discover()?;
    let mut removed_binary = false;
    let mut removed_skills = false;
    let mut removed_data = false;

    let remove_binary = options
        .remove_binary
        .unwrap_or_else(|| options.yes || prompt::confirm_step("Remove installed binary?", true, options.yes));
    if remove_binary {
        if state.binary_path.exists() {
            std::fs::remove_file(&state.binary_path)?;
            println!("Removed {}", state.binary_path.display());
        } else {
            println!("Binary not found at {}", state.binary_path.display());
        }
        removed_binary = true;
    }

    let skill_dirs = state.skill_install_dirs();
    if !skill_dirs.is_empty() {
        let remove_skills = options.remove_skills.unwrap_or_else(|| {
            options.yes
                || prompt::confirm_step(
                    &format!("Remove {} skill install(s)?", skill_dirs.len()),
                    false,
                    options.yes,
                )
        });
        if remove_skills {
            for dir in &skill_dirs {
                if dir.exists() {
                    std::fs::remove_dir_all(dir)?;
                    println!("Removed {}", dir.display());
                }
            }
            removed_skills = true;
        }
    }

    let config_dir = crate::config::AppConfig::config_dir()?;
    let remove_data = options.remove_data.unwrap_or_else(|| {
        options.yes || prompt::confirm_step("Remove config, database, and chat sessions?", false, options.yes)
    });
    if remove_data {
        for entry in ["config.toml", "generations.db", "install.toml"] {
            let path = config_dir.join(entry);
            if path.exists() {
                std::fs::remove_file(&path)?;
                println!("Removed {}", path.display());
            }
        }
        let sessions = config_dir.join("chat_sessions");
        if sessions.exists() {
            std::fs::remove_dir_all(&sessions)?;
            println!("Removed {}", sessions.display());
        }
        removed_data = true;
    } else if (removed_binary || removed_skills) && InstallState::path()?.exists() {
        std::fs::remove_file(InstallState::path()?)?;
    }

    if !removed_binary && !removed_skills && !removed_data {
        println!("Uninstall cancelled; nothing removed.");
    } else {
        println!("Uninstall complete.");
    }
    Ok(())
}

pub struct UninstallOptions {
    pub yes: bool,
    pub remove_binary: Option<bool>,
    pub remove_skills: Option<bool>,
    pub remove_data: Option<bool>,
}
