use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pi_ai::Message;
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub model: String,
    pub thinking: bool,
    pub messages: Vec<Message>,
}

impl ChatSession {
    pub fn new(model: &str, thinking: bool) -> Self {
        let now = pi_ai::now_ms();
        Self {
            id: new_id(),
            created_ms: now,
            updated_ms: now,
            model: model.to_string(),
            thinking,
            messages: Vec::new(),
        }
    }

    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.updated_ms = pi_ai::now_ms();
    }
}

#[derive(Debug, Clone)]
pub struct ChatSessionSummary {
    pub id: String,
    pub updated_ms: i64,
    pub model: String,
    pub thinking: bool,
    pub turns: usize,
    pub first_message: String,
}

pub fn sessions_dir() -> Result<PathBuf> {
    Ok(AppConfig::config_dir()?.join("chat_sessions"))
}

pub fn save_session(session: &ChatSession) -> Result<PathBuf> {
    save_session_to_dir(&sessions_dir()?, session)
}

pub fn load_session(id: &str) -> Result<ChatSession> {
    load_session_from_dir(&sessions_dir()?, id)
}

pub fn list_sessions() -> Result<Vec<ChatSessionSummary>> {
    list_sessions_from_dir(&sessions_dir()?)
}

pub fn save_session_to_dir(dir: &Path, session: &ChatSession) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let path = dir.join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session).context("serialize chat session")?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn load_session_from_dir(dir: &Path, id: &str) -> Result<ChatSession> {
    let path = dir.join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).context("parse chat session")
}

pub fn list_sessions_from_dir(dir: &Path) -> Result<Vec<ChatSessionSummary>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_str::<ChatSession>(&text) else {
            continue;
        };
        out.push(ChatSessionSummary {
            id: session.id,
            updated_ms: session.updated_ms,
            model: session.model,
            thinking: session.thinking,
            turns: session.messages.len(),
            first_message: first_user_message(&session.messages),
        });
    }
    out.sort_by_key(|summary| std::cmp::Reverse(summary.updated_ms));
    Ok(out)
}

fn first_user_message(messages: &[Message]) -> String {
    messages
        .iter()
        .find_map(|message| match message {
            Message::User { content, .. } => content.iter().find_map(|content| content.as_text().map(str::to_string)),
            _ => None,
        })
        .unwrap_or_default()
}

fn new_id() -> String {
    let suffix: u32 = rand::thread_rng().r#gen();
    format!("{:x}-{suffix:08x}", pi_ai::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::Message;

    #[test]
    fn session_roundtrip_keeps_messages() {
        let dir = std::env::temp_dir().join(format!("agnes-chat-session-{}", pi_ai::now_ms()));
        let mut session = ChatSession::new("agnes-2.0-flash", true);
        session.messages.push(Message::user_text("hello"));

        save_session_to_dir(&dir, &session).unwrap();
        let loaded = load_session_from_dir(&dir, &session.id).unwrap();

        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.messages.len(), 1);
        assert!(loaded.thinking);
        let _ = std::fs::remove_dir_all(dir);
    }
}
