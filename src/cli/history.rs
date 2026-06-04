use anyhow::Result;
use clap::{Args, Subcommand};

use crate::db::Database;

#[derive(Args)]
pub struct HistoryCmd {
    #[command(subcommand)]
    pub action: HistoryAction,
}

#[derive(Subcommand)]
pub enum HistoryAction {
    /// List recent generations
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a generation record by id
    Show { id: i64 },
}

#[derive(Args)]
pub struct AssetCmd {
    #[command(subcommand)]
    pub action: AssetAction,
}

#[derive(Subcommand)]
pub enum AssetAction {
    /// List assets in the library
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show an asset (accepts asset://id or bare id)
    Show { id: String },
}

pub fn run_history(cmd: HistoryCmd) -> Result<()> {
    let db = Database::open()?;
    match cmd.action {
        HistoryAction::List { limit } => {
            let rows = db.list_generations(limit)?;
            if rows.is_empty() {
                println!("No generations recorded.");
                return Ok(());
            }
            for row in rows {
                let prompt = row.prompt.unwrap_or_default();
                let prompt_short = if prompt.chars().count() > 40 {
                    format!("{}…", prompt.chars().take(39).collect::<String>())
                } else {
                    prompt
                };
                println!(
                    "#{:<6} {:<6} {:<20} asset={}",
                    row.id,
                    row.kind,
                    prompt_short,
                    row.asset_id.unwrap_or_else(|| "-".into())
                );
            }
        }
        HistoryAction::Show { id } => {
            let row = db.get_generation(id)?;
            println!("{}", serde_json::to_string_pretty(&row)?);
        }
    }
    Ok(())
}

pub fn run_asset(cmd: AssetCmd) -> Result<()> {
    let db = Database::open()?;
    match cmd.action {
        AssetAction::List { limit } => {
            let rows = db.list_assets(limit)?;
            if rows.is_empty() {
                println!("No assets recorded.");
                return Ok(());
            }
            for row in rows {
                println!(
                    "{:<22} {:<6} {:<8} {}",
                    row.asset_uri,
                    row.kind,
                    row.ratio.unwrap_or_default(),
                    row.remote_url
                );
            }
        }
        AssetAction::Show { id } => {
            let asset = db.get_asset(&id)?;
            println!("{}", serde_json::to_string_pretty(&asset)?);
        }
    }
    Ok(())
}
