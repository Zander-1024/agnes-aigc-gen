use anyhow::Result;
use clap::{Args, Subcommand};
use log;

use crate::api::{ApiClient, refresh_video_task, wait_video_task};
use crate::config::AppConfig;
use crate::db::Database;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct TaskCmd {
    #[command(subcommand)]
    pub action: TaskAction,
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// List recent video tasks (default 10); refreshes in-progress tasks from API
    List {
        #[arg(short = 'n', long = "limit", default_value_t = 10)]
        limit: usize,
    },
    /// Query a video task once (refresh from API, update local record)
    Show {
        /// Local id (e.g. `3`), `#3`, or vendor task id
        task_ref: String,
        #[arg(long = "output-format", default_value = "json")]
        output_format: String,
    },
    /// Poll until a video task completes (same as `video --task-id`)
    Wait {
        /// Local id (e.g. `3`), `#3`, or vendor task id
        task_ref: String,
        #[arg(long = "output-dir")]
        output_dir: Option<String>,
        #[arg(long = "save")]
        save: bool,
        #[arg(long = "retries")]
        retries: Option<u32>,
        #[arg(long = "output-format", default_value = "json")]
        output_format: String,
    },
}

pub fn run(cmd: TaskCmd) -> Result<()> {
    match cmd.action {
        TaskAction::List { limit } => run_list(limit),
        TaskAction::Show { task_ref, output_format } => run_show(&task_ref, output_format),
        TaskAction::Wait { task_ref, output_dir, save, retries, output_format } => {
            run_wait(task_ref, output_dir, save, retries, output_format)
        }
    }
}

/// Resolve local id or vendor task id for CLI commands.
pub fn resolve_task_ref(reference: &str) -> Result<String> {
    Database::open()?.resolve_video_task_ref(reference)
}

fn run_list(limit: usize) -> Result<()> {
    let db = Database::open()?;
    let pending = db.list_video_tasks(limit)?;
    if pending.is_empty() {
        println!("No video tasks recorded.");
        return Ok(());
    }

    if let Ok(api) = ApiClient::from_config(AppConfig::load()?) {
        for row in pending.iter().filter(|r| r.phase == "processing") {
            if let Err(err) = refresh_video_task(&api, &row.task_id) {
                log::warn!("refresh task {} (#{}): {err}", row.task_id, row.id);
            }
        }
    }

    let rows = db.list_video_tasks(limit)?;
    println!(
        "{:<4} {:<10} {:<11} {:<10} {}",
        "ID", "TASK_ID", "PHASE", "PROMPT", "URI"
    );
    for row in rows {
        let prompt = truncate_display(&row.prompt.unwrap_or_default(), 10);
        let task_id = truncate_display(&row.task_id, 10);
        let uri = row.uri.unwrap_or_else(|| "-".into());
        println!("{:<4} {:<10} {:<11} {:<10} {}", row.id, task_id, row.phase, prompt, uri);
    }
    Ok(())
}

fn truncate_display(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".chars().take(max_chars).collect();
    }
    format!("{}...", chars.into_iter().take(max_chars - 3).collect::<String>())
}

fn run_show(task_ref: &str, output_format: String) -> Result<()> {
    let task_id = resolve_task_ref(task_ref)?;
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    let record = refresh_video_task(&api, &task_id)?;
    print_task_record(&record, output_format)
}

fn run_wait(
    task_ref: String,
    output_dir: Option<String>,
    save: bool,
    retries: Option<u32>,
    output_format: String,
) -> Result<()> {
    let task_id = resolve_task_ref(&task_ref)?;
    let cfg = AppConfig::load()?;
    let format = parse_output_format(&output_format)?;
    let api = ApiClient::with_overrides(cfg, output_dir, None, retries)?;
    wait_video_task(&api, &task_id, save, format)?;
    Ok(())
}

fn print_task_record(record: &crate::db::VideoTaskRecord, output_format: String) -> Result<()> {
    match output_format.to_lowercase().as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(record)?),
        "plain" => {
            println!("id={}", record.id);
            println!("task_id={}", record.task_id);
            println!("phase={}", record.phase);
            println!("status={}", record.status);
            if let Some(p) = record.progress {
                println!("progress={p}");
            }
            if let Some(ref uri) = record.uri {
                println!("uri={uri}");
            }
            if let Some(ref asset) = record.asset_uri {
                println!("asset_uri={asset}");
            }
        }
        other => anyhow::bail!("unknown output format: {other}"),
    }
    Ok(())
}

fn parse_output_format(s: &str) -> Result<OutputFormat> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "plain" => Ok(OutputFormat::Plain),
        other => anyhow::bail!("unknown output format: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_display_short() {
        assert_eq!(truncate_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_display_long() {
        assert_eq!(truncate_display("abcdefghijk", 10), "abcdefg...");
    }
}
