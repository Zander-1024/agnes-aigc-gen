use anyhow::Result;
use clap::{Args, Subcommand};

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
    /// List recent video tasks (default 10)
    List {
        #[arg(short = 'n', long = "limit", default_value_t = 10)]
        limit: usize,
    },
    /// Query a video task once (refresh from API, update local record)
    Show {
        task_id: String,
        #[arg(long = "output-format", default_value = "json")]
        output_format: String,
    },
    /// Poll until a video task completes (same as `video --task-id`)
    Wait {
        task_id: String,
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
        TaskAction::Show { task_id, output_format } => run_show(&task_id, output_format),
        TaskAction::Wait {
            task_id,
            output_dir,
            save,
            retries,
            output_format,
        } => run_wait(task_id, output_dir, save, retries, output_format),
    }
}

fn run_list(limit: usize) -> Result<()> {
    let db = Database::open()?;
    let rows = db.list_video_tasks(limit)?;
    if rows.is_empty() {
        println!("No video tasks recorded.");
        return Ok(());
    }
    println!("{:<28} {:<11} {:<12} {}", "TASK_ID", "PHASE", "PROMPT", "URI");
    for row in rows {
        let prompt = row.prompt.unwrap_or_default();
        let prompt_short = if prompt.chars().count() > 36 {
            format!("{}…", prompt.chars().take(35).collect::<String>())
        } else {
            prompt
        };
        let uri = row.uri.unwrap_or_else(|| "-".into());
        println!(
            "{:<28} {:<11} {:<12} {}",
            row.task_id, row.phase, prompt_short, uri
        );
    }
    Ok(())
}

fn run_show(task_id: &str, output_format: String) -> Result<()> {
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    let record = refresh_video_task(&api, task_id)?;
    print_task_record(&record, output_format)
}

fn run_wait(
    task_id: String,
    output_dir: Option<String>,
    save: bool,
    retries: Option<u32>,
    output_format: String,
) -> Result<()> {
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
