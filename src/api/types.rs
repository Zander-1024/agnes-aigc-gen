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
    pub extra_body: Option<ExtraBodyVideo>,
}

#[derive(Debug, Serialize)]
pub struct ExtraBodyVideo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VideoTaskResponse {
    pub id: Option<String>,
    pub task_id: Option<String>,
    pub status: String,
    pub video_url: Option<String>,
    #[serde(default)]
    pub remixed_from_video_id: Option<String>,
    pub error: Option<serde_json::Value>,
}

impl VideoTaskResponse {
    pub fn task_id(&self) -> Option<String> {
        self.task_id.clone().or_else(|| self.id.clone())
    }

    pub fn result_url(&self) -> Option<String> {
        self.video_url.clone().or_else(|| self.remixed_from_video_id.clone())
    }
}
