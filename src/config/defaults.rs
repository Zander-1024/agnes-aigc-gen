pub const DEFAULT_BASE_URL: &str = "https://apihub.agnes-ai.com";
pub const DEFAULT_TEXT_MODEL: &str = "agnes-3.0-flash";
pub const DEFAULT_IMAGE_MODEL: &str = "agnes-image-2.5-flash";
pub const DEFAULT_VIDEO_MODEL: &str = "agnes-video-2.5";
pub const DEFAULT_CHAT_THINKING: bool = true;
pub const DEFAULT_CHAT_CONTEXT_TOKENS: u32 = 524_288;
pub const DEFAULT_CHAT_MAX_OUTPUT_TOKENS: u32 = 65_536;
/// Current working directory when generating or downloading outputs.
pub const DEFAULT_OUTPUT_DIR: &str = ".";
pub const DEFAULT_MAX_RETRIES: u32 = 3;

pub const BASE_URL_HELP: &str = "Agnes API gateway (optional trailing /v1 is accepted)";
pub const OUTPUT_DIR_HELP: &str = "`.` = current working directory";
