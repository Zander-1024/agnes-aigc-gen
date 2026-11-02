use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// Vendor task lifecycle status, as returned by the Agnes video API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    /// An unrecognized vendor status string. Preserved verbatim for logging
    /// and persistence; never matched as "completed" or "failed".
    Unknown(String),
}

impl VideoStatus {
    pub fn as_str(&self) -> &str {
        match self {
            VideoStatus::Queued => "queued",
            VideoStatus::InProgress => "in_progress",
            VideoStatus::Completed => "completed",
            VideoStatus::Failed => "failed",
            VideoStatus::Unknown(raw) => raw,
        }
    }

    #[allow(dead_code)]
    pub fn is_terminal(&self) -> bool {
        matches!(self, VideoStatus::Completed | VideoStatus::Failed)
    }
}

impl fmt::Display for VideoStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for VideoStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "queued" => VideoStatus::Queued,
            "in_progress" => VideoStatus::InProgress,
            "completed" => VideoStatus::Completed,
            "failed" => VideoStatus::Failed,
            other => VideoStatus::Unknown(other.to_string()),
        })
    }
}

fn deserialize_video_status<'de, D: Deserializer<'de>>(de: D) -> Result<VideoStatus, D::Error> {
    let raw = String::deserialize(de)?;
    use serde::de::Error as _;
    raw.parse::<VideoStatus>().map_err(D::Error::custom)
}

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
    pub ratio: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoReference {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f64>,
    #[serde(default)]
    pub require_audio: bool,
}

#[derive(Debug, Serialize)]
pub struct VideoCreateRequest {
    pub model: String,
    pub prompt: String,
    pub mode: String,
    pub seconds: String,
    pub size: String,
    pub aspect_ratio: String,
    pub n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_frame: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub audios: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub videos: Vec<VideoReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct VideoTaskResponse {
    pub id: Option<String>,
    pub video_id: Option<String>,
    pub task_id: Option<String>,
    #[serde(deserialize_with = "deserialize_video_status")]
    pub status: VideoStatus,
    #[serde(default)]
    pub progress: Option<i32>,
    pub url: Option<String>,
    pub video_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
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
        let metadata_url = self
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("url"))
            .and_then(serde_json::Value::as_str);
        [self.url.as_deref(), self.video_url.as_deref(), metadata_url]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|url| !url.is_empty())
            .map(str::to_string)
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
    fn image_generation_request_serializes_ratio_and_size_tier() {
        let body = ImageGenerationRequest {
            model: "agnes-image-2.5-flash".into(),
            prompt: "test".into(),
            ratio: "16:9".into(),
            size: "2K".into(),
            extra_body: None,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["ratio"], "16:9");
        assert_eq!(json["size"], "2K");
    }

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

    #[test]
    fn completed_video_reads_top_level_url_and_ignores_source_id() {
        let mut value = serde_json::json!({
            "id": "task_123",
            "status": "completed",
            "progress": 100,
            "internal_status": "pending",
            "url": "https://example.com/result.mp4",
            "remixed_from_video_id": "video_source"
        });
        let task: VideoTaskResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(task.result_url().as_deref(), Some("https://example.com/result.mp4"));
        value.as_object_mut().unwrap().remove("url");
        let task: VideoTaskResponse = serde_json::from_value(value).unwrap();
        assert!(task.result_url().is_none());
    }

    #[test]
    fn completed_video_accepts_metadata_url_from_task_query() {
        let task: VideoTaskResponse = serde_json::from_value(serde_json::json!({
            "id": "task_example",
            "video_id": "task_example",
            "object": "video",
            "status": "completed",
            "progress": 100,
            "metadata": {"url": "https://example.com/result.mp4"}
        }))
        .unwrap();
        assert_eq!(task.result_url().as_deref(), Some("https://example.com/result.mp4"));
    }

    #[test]
    fn completed_video_accepts_both_top_level_url_fields() {
        let task: VideoTaskResponse = serde_json::from_value(serde_json::json!({
            "status": "completed",
            "url": "https://example.com/preferred.mp4",
            "video_url": "https://example.com/alternate.mp4",
            "metadata": {"url": "https://example.com/metadata.mp4"}
        }))
        .unwrap();
        assert_eq!(task.result_url().as_deref(), Some("https://example.com/preferred.mp4"));
    }

    #[test]
    fn completed_video_skips_empty_url_fields_and_trims_result() {
        let mut value = serde_json::json!({
            "status": "completed",
            "url": "  ",
            "video_url": "\n\t",
            "metadata": {"url": "  https://example.com/result.mp4 \n"},
            "remixed_from_video_id": "https://example.com/source.mp4"
        });
        let task: VideoTaskResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(task.result_url().as_deref(), Some("https://example.com/result.mp4"));
        value["video_url"] = serde_json::json!(" https://example.com/video.mp4 ");
        let task: VideoTaskResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(task.result_url().as_deref(), Some("https://example.com/video.mp4"));
        value["video_url"] = serde_json::json!(null);
        for metadata in [serde_json::json!({"url": " \n"}), serde_json::json!({"url": 42}), serde_json::json!({})] {
            value["metadata"] = metadata;
            let task: VideoTaskResponse = serde_json::from_value(value.clone()).unwrap();
            assert!(task.result_url().is_none());
        }
    }

    #[test]
    fn video_status_parses_known_states() {
        assert_eq!("queued".parse::<VideoStatus>().unwrap(), VideoStatus::Queued);
        assert_eq!("in_progress".parse::<VideoStatus>().unwrap(), VideoStatus::InProgress);
        assert_eq!("completed".parse::<VideoStatus>().unwrap(), VideoStatus::Completed);
        assert_eq!("failed".parse::<VideoStatus>().unwrap(), VideoStatus::Failed);
    }

    #[test]
    fn video_status_unknown_preserves_raw_string() {
        let s = "succeeded".parse::<VideoStatus>().unwrap();
        assert_eq!(s.as_str(), "succeeded");
        assert_eq!(s, VideoStatus::Unknown("succeeded".into()));
        assert!(!s.is_terminal());
    }

    #[test]
    fn video_status_is_terminal_only_for_done_states() {
        assert!(!VideoStatus::Queued.is_terminal());
        assert!(!VideoStatus::InProgress.is_terminal());
        assert!(VideoStatus::Completed.is_terminal());
        assert!(VideoStatus::Failed.is_terminal());
        assert!(!VideoStatus::Unknown("succeeded".into()).is_terminal());
    }

    #[test]
    fn video_task_response_deserializes_unknown_status() {
        let task: VideoTaskResponse = serde_json::from_value(serde_json::json!({
            "video_id": "v1",
            "status": "succeeded"
        }))
        .unwrap();
        assert_eq!(task.status, VideoStatus::Unknown("succeeded".into()));
    }
}
