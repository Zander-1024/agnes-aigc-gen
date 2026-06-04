pub const DEFAULT_BASE_URL: &str = "https://apihub.agnes-ai.com/v1";
pub const DEFAULT_TEXT_MODEL: &str = "agnes-2.0-flash";
pub const DEFAULT_IMAGE_MODEL: &str = "agnes-image-2.1-flash";
pub const DEFAULT_VIDEO_MODEL: &str = "agnes-video-v2.0";
/// Current working directory when generating or downloading outputs.
pub const DEFAULT_OUTPUT_DIR: &str = ".";
pub const DEFAULT_MAX_RETRIES: u32 = 3;

pub const BASE_URL_HELP: &str = "Agnes API gateway (OpenAI-compatible)";
pub const OUTPUT_DIR_HELP: &str = "`.` = current working directory";
