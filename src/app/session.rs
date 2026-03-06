use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{app::AppResult, llm::ChatMessage};

use super::ToolLogSection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub messages: Vec<String>,
    #[serde(default)]
    pub chat_history: Vec<ChatMessage>,
    #[serde(default)]
    pub tool_logs: Vec<ToolLogSection>,
    #[serde(default)]
    pub show_thinking: bool,
    #[serde(default)]
    pub show_tool_details: bool,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct CurrentSessionState {
    current_session_id: String,
}

impl SessionStore {
    pub fn new(root: impl Into<PathBuf>) -> AppResult<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("sessions"))?;
        Ok(Self { root })
    }

    pub fn load_current(&self) -> AppResult<Option<SessionData>> {
        let Some(id) = self.current_session_id()? else {
            return Ok(None);
        };

        self.load_session(&id)
    }

    pub fn save_session(&self, session: &SessionData) -> AppResult<()> {
        let path = self.session_file_path(&session.id);
        let content = serde_json::to_string_pretty(session)?;
        fs::write(path, content)?;
        self.set_current_session_id(&session.id)?;
        Ok(())
    }

    pub fn load_session(&self, id: &str) -> AppResult<Option<SessionData>> {
        let path = self.session_file_path(id);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&content)?))
    }

    pub fn rename_session(&self, id: &str, title: &str) -> AppResult<Option<SessionData>> {
        let Some(mut session) = self.load_session(id)? else {
            return Ok(None);
        };

        session.title = normalize_title(Some(title));
        session.updated_at = unix_timestamp();
        self.save_session(&session)?;
        Ok(Some(session))
    }

    pub fn list_sessions(&self) -> AppResult<Vec<SessionSummary>> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(self.sessions_dir())? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let content = fs::read_to_string(&path)?;
            let session = serde_json::from_str::<SessionData>(&content)?;
            sessions.push(SessionSummary {
                id: session.id,
                title: session.title,
                updated_at: session.updated_at,
            });
        }

        sessions.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(sessions)
    }

    pub fn new_session(&self, title: Option<&str>, welcome_messages: Vec<String>) -> SessionData {
        let now = unix_timestamp();
        SessionData {
            id: session_id(),
            title: normalize_title(title),
            created_at: now,
            updated_at: now,
            messages: welcome_messages,
            chat_history: Vec::new(),
            tool_logs: Vec::new(),
            show_thinking: false,
            show_tool_details: false,
        }
    }

    pub fn resolve_session_id(&self, raw: &str) -> AppResult<Option<String>> {
        let sessions = self.list_sessions()?;
        let mut matches = sessions
            .into_iter()
            .filter(|session| session.id == raw || session.id.starts_with(raw))
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(matches.into_iter().next().map(|session| session.id))
    }

    fn current_session_id(&self) -> AppResult<Option<String>> {
        let path = self.root.join("current.json");
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        let state = serde_json::from_str::<CurrentSessionState>(&content)?;
        Ok(Some(state.current_session_id))
    }

    fn set_current_session_id(&self, id: &str) -> AppResult<()> {
        let path = self.root.join("current.json");
        let content = serde_json::to_string_pretty(&CurrentSessionState {
            current_session_id: id.to_string(),
        })?;
        fs::write(path, content)?;
        Ok(())
    }

    fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    fn session_file_path(&self, id: &str) -> PathBuf {
        self.sessions_dir().join(format!("{id}.json"))
    }
}

fn normalize_title(title: Option<&str>) -> String {
    let trimmed = title.unwrap_or_default().trim();
    if trimmed.is_empty() {
        format!("Session {}", unix_timestamp())
    } else {
        trimmed.to_string()
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn session_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

#[allow(dead_code)]
fn _ensure_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}