use std::io::{self, IsTerminal, Write};

use anyhow::{Result, bail};

use super::skill::SKILL_NAME;
use super::state::InstallState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillUpdateChoice {
    Previous,
    DefaultOnly,
    Custom(Vec<String>),
    Skip,
}

pub fn is_tty() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn prompt_skill_update(state: &InstallState) -> Result<SkillUpdateChoice> {
    if !is_tty() {
        return Ok(SkillUpdateChoice::Skip);
    }

    let recommended = state.recommended_skill_parents();
    println!("Update skill files?");
    println!("  [1] Yes — previous locations (recommended)");
    for parent in &recommended {
        println!("      {parent}/{SKILL_NAME}");
    }
    println!("  [2] Yes — default location only (~/.agents/skills)");
    println!("  [3] Yes — custom parent dir(s), comma-separated");
    println!("  [4] No — binary only");
    print!("Choose [1-4, default 4]: ");
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    match line.trim() {
        "" | "4" => Ok(SkillUpdateChoice::Skip),
        "1" => Ok(SkillUpdateChoice::Previous),
        "2" => Ok(SkillUpdateChoice::DefaultOnly),
        "3" => {
            print!("Parent dir(s): ");
            io::stdout().flush()?;
            let mut custom = String::new();
            io::stdin().read_line(&mut custom)?;
            let parents: Vec<String> = custom
                .split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect();
            if parents.is_empty() {
                bail!("no custom skill directories provided");
            }
            Ok(SkillUpdateChoice::Custom(parents))
        }
        other => bail!("invalid choice: {other}"),
    }
}

pub fn confirm_step(prompt: &str, default_yes: bool, auto_yes: bool) -> bool {
    if auto_yes {
        return default_yes;
    }
    if !is_tty() {
        return default_yes;
    }
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("{prompt} [{hint}] ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default_yes;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    }
}
