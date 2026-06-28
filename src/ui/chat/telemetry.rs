#[derive(Debug, Clone, Default)]
pub(crate) struct SessionTelemetry {
    pub model_label: String,
    pub directory_label: String,
    pub approval_label: String,
    pub thinking_enabled: bool,
    pub context_tokens: u32,
    pub max_output_tokens: u32,
    pub last_prompt_tokens: u64,
    pub last_cache_read: u64,
    pub estimated_session_tokens: u64,
}
