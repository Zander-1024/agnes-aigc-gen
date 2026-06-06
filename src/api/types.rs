use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ImageGenerationResponse {
    pub data: Vec<ImageDataItem>,
}

#[derive(Debug, Deserialize)]
pub struct ImageDataItem {
    pub url: Option<String>,
    pub b64_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImageGenerationRequest {
    pub model: String,
    pub prompt: String,
    pub size: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<ExtraBodyImage>,
}

#[derive(Debug, Serialize)]
pub struct ExtraBodyImage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct VideoCreateRequest {
    pub model: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<ExtraBodyVideo>,
}

#[derive(Debug, Serialize)]
pub struct ExtraBodyVideo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct VideoTaskResponse {
    pub id: Option<String>,
    pub video_id: Option<String>,
    pub task_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub progress: Option<i32>,
    pub video_url: Option<String>,
    #[serde(default)]
    pub remixed_from_video_id: Option<String>,
    pub error: Option<serde_json::Value>,
}

impl VideoTaskResponse {
    pub fn query_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        push_unique(&mut ids, self.video_id.as_deref());
        push_unique(&mut ids, self.id.as_deref());
        push_unique(&mut ids, self.task_id.as_deref());
        ids
    }

    pub fn result_url(&self) -> Option<String> {
        self.video_url.clone().or_else(|| self.remixed_from_video_id.clone())
    }
}

fn push_unique(ids: &mut Vec<String>, id: Option<&str>) {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };
    if !ids.iter().any(|existing| existing == id) {
        ids.push(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_task_query_id_prefers_video_id() {
        let task: VideoTaskResponse = serde_json::from_value(serde_json::json!({
            "video_id": "video_123",
            "id": "task_123",
            "task_id": "legacy_123",
            "status": "queued"
        }))
        .unwrap();

        assert_eq!(task.query_ids(), ["video_123", "task_123", "legacy_123"]);
    }

    #[test]
    fn video_task_query_id_falls_back_to_legacy_ids() {
        let task: VideoTaskResponse = serde_json::from_value(serde_json::json!({
            "task_id": "task_123",
            "status": "queued"
        }))
        .unwrap();

        assert_eq!(task.query_ids(), ["task_123"]);
    }
}
