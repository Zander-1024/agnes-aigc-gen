use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use pi_agent::{AgentTool, AgentToolResult};
use serde_json::{Value, json};

use crate::api::{
    ApiClient, ImageRequest, VideoRequest, generate_image, generate_video, refresh_video_task, wait_video_task,
};
use crate::config::AppConfig;
use crate::db::Database;
use crate::output::{ImageBatchItem, MAX_IMAGE_BATCH_COUNT, OutputFormat};
use crate::ratio::{AspectRatio, validate_frame_rate};

pub fn default_agent_tools() -> Vec<Arc<dyn AgentTool>> {
    let mut tools: Vec<Arc<dyn AgentTool>> = pi_agent::tools::default_tools()
        .into_iter()
        .map(|tool| Arc::new(GuardedTool { inner: tool }) as Arc<dyn AgentTool>)
        .collect();
    tools.extend([
        Arc::new(AgnesGenerateImageTool) as Arc<dyn AgentTool>,
        Arc::new(AgnesSubmitVideoTool),
        Arc::new(AgnesTaskListTool),
        Arc::new(AgnesTaskShowTool),
        Arc::new(AgnesTaskWaitTool),
        Arc::new(AgnesAssetListTool),
        Arc::new(AgnesAssetShowTool),
        Arc::new(AgnesHistoryListTool),
        Arc::new(AgnesHistoryShowTool),
        Arc::new(LoadSkillTool),
    ]);
    tools
}

struct GuardedTool {
    inner: Arc<dyn AgentTool>,
}

#[async_trait]
impl AgentTool for GuardedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn label(&self) -> &str {
        self.inner.label()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, tool_call_id: &str, args: Value) -> std::result::Result<AgentToolResult, String> {
        self.inner.execute(tool_call_id, args).await
    }
}

pub struct AgnesGenerateImageTool;
pub struct AgnesSubmitVideoTool;
pub struct AgnesTaskListTool;
pub struct AgnesTaskShowTool;
pub struct AgnesTaskWaitTool;
pub struct AgnesAssetListTool;
pub struct AgnesAssetShowTool;
pub struct AgnesHistoryListTool;
pub struct AgnesHistoryShowTool;
pub struct LoadSkillTool;

#[async_trait]
impl AgentTool for AgnesGenerateImageTool {
    fn name(&self) -> &str {
        "agnes_generate_image"
    }

    fn description(&self) -> &str {
        "Generate one or more images with Agnes Image. Inputs may be local path, HTTPS URL, asset://, base64, or data URI."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "ratio": {"type": "string", "default": "1:1"},
                "inputs": {"type": "array", "items": {"type": "string"}, "default": []},
                "count": {"type": "integer", "minimum": 1, "maximum": 4, "default": 1},
                "seed": {"type": "integer", "minimum": 0, "maximum": 999},
                "save": {"type": "boolean", "default": false}
            },
            "required": ["prompt"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> std::result::Result<AgentToolResult, String> {
        spawn_tool(move || generate_image_tool(args)).await
    }
}

#[async_trait]
impl AgentTool for AgnesSubmitVideoTool {
    fn name(&self) -> &str {
        "agnes_submit_video"
    }

    fn description(&self) -> &str {
        "Submit an Agnes Video task asynchronously. Video image inputs must be HTTPS URLs or asset:// references."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "negative_prompt": {"type": "string"},
                "ratio": {"type": "string", "default": "16:9"},
                "duration": {"type": "number", "default": 5},
                "frame_rate": {"type": "integer", "default": 24},
                "images": {"type": "array", "items": {"type": "string"}, "default": []},
                "seed": {"type": "integer", "minimum": 0, "maximum": 999}
            },
            "required": ["prompt"]
        })
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn execute(&self, _tool_call_id: &str, args: Value) -> std::result::Result<AgentToolResult, String> {
        spawn_tool(move || submit_video_tool(args)).await
    }
}

macro_rules! simple_tool {
    ($ty:ident, $name:literal, $description:literal, $schema:expr, $handler:ident) => {
        #[async_trait]
        impl AgentTool for $ty {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $description
            }

            fn parameters(&self) -> Value {
                $schema
            }

            fn requires_permission(&self) -> bool {
                true
            }

            async fn execute(&self, _tool_call_id: &str, args: Value) -> std::result::Result<AgentToolResult, String> {
                spawn_tool(move || $handler(args)).await
            }
        }
    };
}

simple_tool!(
    AgnesTaskListTool,
    "agnes_task_list",
    "List recent Agnes async video tasks and refresh in-progress tasks when possible.",
    json!({"type": "object", "properties": {"limit": {"type": "integer", "default": 10}}}),
    task_list_tool
);

simple_tool!(
    AgnesTaskShowTool,
    "agnes_task_show",
    "Show and refresh one Agnes async video task by local id, #id, or vendor task_id.",
    json!({"type": "object", "properties": {"task_ref": {"type": "string"}}, "required": ["task_ref"]}),
    task_show_tool
);

simple_tool!(
    AgnesTaskWaitTool,
    "agnes_task_wait",
    "Wait for one Agnes async video task to complete. This may block for minutes.",
    json!({"type": "object", "properties": {"task_ref": {"type": "string"}, "save": {"type": "boolean", "default": false}}, "required": ["task_ref"]}),
    task_wait_tool
);

simple_tool!(
    AgnesAssetListTool,
    "agnes_asset_list",
    "List recent Agnes asset:// records.",
    json!({"type": "object", "properties": {"limit": {"type": "integer", "default": 20}}}),
    asset_list_tool
);

simple_tool!(
    AgnesAssetShowTool,
    "agnes_asset_show",
    "Show one Agnes asset:// record.",
    json!({"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}),
    asset_show_tool
);

simple_tool!(
    AgnesHistoryListTool,
    "agnes_history_list",
    "List recent Agnes generation history records.",
    json!({"type": "object", "properties": {"limit": {"type": "integer", "default": 20}}}),
    history_list_tool
);

simple_tool!(
    AgnesHistoryShowTool,
    "agnes_history_show",
    "Show one Agnes generation history record.",
    json!({"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}),
    history_show_tool
);

simple_tool!(
    LoadSkillTool,
    "load_skill",
    "Load an installed skill's SKILL.md by name.",
    json!({"type": "object", "properties": {"name": {"type": "string"}, "max_chars": {"type": "integer", "default": 30000}}, "required": ["name"]}),
    load_skill_tool
);

async fn spawn_tool<F>(f: F) -> std::result::Result<AgentToolResult, String>
where
    F: FnOnce() -> Result<Value> + Send + 'static,
{
    let value = tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| format!("tool task failed: {err}"))?
        .map_err(|err| format!("{err:#}"))?;
    let text = serde_json::to_string_pretty(&value).map_err(|err| err.to_string())?;
    Ok(AgentToolResult::text(text))
}

fn generate_image_tool(args: Value) -> Result<Value> {
    let prompt = required_str(&args, "prompt")?.to_string();
    let ratio = optional_str(&args, "ratio").unwrap_or("1:1");
    let count = optional_u32(&args, "count").unwrap_or(1);
    anyhow::ensure!(
        (1..=MAX_IMAGE_BATCH_COUNT).contains(&count),
        "count must be 1-{MAX_IMAGE_BATCH_COUNT}"
    );
    let seed = optional_u32(&args, "seed");
    anyhow::ensure!(count == 1 || seed.is_none(), "seed cannot be used with count > 1");
    let inputs = optional_string_array(&args, "inputs")?;
    let save = optional_bool(&args, "save").unwrap_or(false);
    let ratio = AspectRatio::parse(ratio)?;
    let cfg = AppConfig::load()?;
    let mut results = Vec::new();
    for _ in 0..count {
        let api = ApiClient::with_overrides(cfg.clone(), None, Some(save), None)?;
        match generate_image(
            &api,
            ImageRequest {
                prompt: prompt.clone(),
                ratio: ratio.clone(),
                inputs: inputs.clone(),
                seed,
                output_dir: None,
                save_local: save,
                max_retries: None,
                output_format: OutputFormat::Json,
                quiet: true,
            },
        ) {
            Ok(result) => results.push(ImageBatchItem::from_result(result)),
            Err(err) => results.push(ImageBatchItem::failure(format!("{err:#}"))),
        }
    }
    if count == 1 {
        serde_json::to_value(&results[0]).context("serialize image result")
    } else {
        Ok(json!({ "results": results }))
    }
}

fn submit_video_tool(args: Value) -> Result<Value> {
    let prompt = required_str(&args, "prompt")?.to_string();
    let negative_prompt = optional_str(&args, "negative_prompt").map(str::to_string);
    let ratio = AspectRatio::parse(optional_str(&args, "ratio").unwrap_or("16:9"))?;
    let duration = optional_f64(&args, "duration").unwrap_or(5.0);
    let frame_rate = optional_u32(&args, "frame_rate").unwrap_or(24);
    validate_frame_rate(frame_rate)?;
    let images = optional_string_array(&args, "images")?;
    let seed = optional_u32(&args, "seed");
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    generate_video(
        &api,
        VideoRequest {
            prompt,
            negative_prompt,
            seed,
            ratio,
            duration,
            frame_rate,
            images,
            task_id: None,
            async_mode: true,
            output_dir: None,
            save_local: false,
            max_retries: None,
            output_format: OutputFormat::Json,
            quiet: true,
        },
    )?;
    let record = api
        .db
        .list_video_tasks(1)?
        .into_iter()
        .next()
        .context("video task was submitted but no local record was found")?;
    serde_json::to_value(record).context("serialize video task")
}

fn task_list_tool(args: Value) -> Result<Value> {
    let limit = optional_usize(&args, "limit").unwrap_or(10);
    let db = Database::open()?;
    if let Ok(api) = ApiClient::from_config(AppConfig::load()?) {
        for row in db
            .list_video_tasks(limit)?
            .iter()
            .filter(|row| row.phase == "processing")
        {
            let _ = refresh_video_task(&api, &row.task_id);
        }
    }
    serde_json::to_value(db.list_video_tasks(limit)?).context("serialize task list")
}

fn task_show_tool(args: Value) -> Result<Value> {
    let task_ref = required_str(&args, "task_ref")?;
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    let task_id = Database::open()?.resolve_video_task_ref(task_ref)?;
    serde_json::to_value(refresh_video_task(&api, &task_id)?).context("serialize task")
}

fn task_wait_tool(args: Value) -> Result<Value> {
    let task_ref = required_str(&args, "task_ref")?;
    let save = optional_bool(&args, "save").unwrap_or(false);
    let cfg = AppConfig::load()?;
    let api = ApiClient::from_config(cfg)?;
    let task_id = Database::open()?.resolve_video_task_ref(task_ref)?;
    let result = wait_video_task(&api, &task_id, save, OutputFormat::Json)?;
    serde_json::to_value(result).context("serialize task wait result")
}

fn asset_list_tool(args: Value) -> Result<Value> {
    let limit = optional_usize(&args, "limit").unwrap_or(20);
    serde_json::to_value(Database::open()?.list_assets(limit)?).context("serialize assets")
}

fn asset_show_tool(args: Value) -> Result<Value> {
    let id = required_str(&args, "id")?;
    serde_json::to_value(Database::open()?.get_asset(id)?).context("serialize asset")
}

fn history_list_tool(args: Value) -> Result<Value> {
    let limit = optional_usize(&args, "limit").unwrap_or(20);
    serde_json::to_value(Database::open()?.list_generations(limit)?).context("serialize history")
}

fn history_show_tool(args: Value) -> Result<Value> {
    let id = required_i64(&args, "id")?;
    serde_json::to_value(Database::open()?.get_generation(id)?).context("serialize generation")
}

fn load_skill_tool(args: Value) -> Result<Value> {
    let name = required_str(&args, "name")?;
    let max_chars = optional_usize(&args, "max_chars").unwrap_or(30_000);
    let path = find_skill_path(name).with_context(|| format!("skill not found: {name}"))?;
    let mut body = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    if body.chars().count() > max_chars {
        body = format!(
            "{}\n...(truncated, {} chars total)",
            body.chars().take(max_chars).collect::<String>(),
            body.chars().count()
        );
    }
    Ok(json!({
        "name": name,
        "path": path.display().to_string(),
        "content": body
    }))
}

pub fn list_available_skills() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for root in skill_roots() {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            if path.exists()
                && let Some(name) = entry.file_name().to_str()
            {
                out.push((name.to_string(), path));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn find_skill_path(name: &str) -> Option<PathBuf> {
    list_available_skills()
        .into_iter()
        .find(|(skill_name, _)| skill_name == name)
        .map(|(_, path)| path)
}

fn skill_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("skills"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".agents/skills"));
        roots.push(home.join(".codex/skills"));
        roots.push(home.join(".cursor/skills"));
        roots.push(home.join(".claude/skills"));
        roots.push(home.join(".openclaw/skills"));
        roots.push(home.join(".hermes/skills"));
    }
    roots
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("missing {key}"))
}

fn required_i64(args: &Value, key: &str) -> Result<i64> {
    args.get(key)
        .and_then(Value::as_i64)
        .with_context(|| format!("missing {key}"))
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn optional_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(Value::as_u64).map(|value| value as u32)
}

fn optional_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|value| value as usize)
}

fn optional_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn optional_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn optional_string_array(args: &Value, key: &str) -> Result<Vec<String>> {
    let Some(raw) = args.get(key) else {
        return Ok(Vec::new());
    };
    if let Some(value) = raw.as_str() {
        return Ok(value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect());
    }
    let items = raw
        .as_array()
        .with_context(|| format!("{key} must be an array of strings"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .with_context(|| format!("{key} must only contain strings"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tools_include_agnes_media_tools() {
        let tools = default_agent_tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();

        assert!(names.contains(&"agnes_generate_image"));
        assert!(names.contains(&"agnes_submit_video"));
        assert!(names.contains(&"load_skill"));
    }

    #[test]
    fn all_tools_are_permission_guarded() {
        for tool in default_agent_tools() {
            assert!(tool.requires_permission(), "{} should go through approval", tool.name());
        }
    }

    #[test]
    fn submit_video_tool_schema_requires_prompt() {
        let tool = AgnesSubmitVideoTool;
        let schema = tool.parameters();

        assert_eq!(schema["required"][0], "prompt");
        assert!(schema["properties"].get("duration").is_some());
    }
}
