use std::{collections::HashMap, fs, path::Path};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::app::AppResult;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenAiCompatible,
    Anthropic,
    AliyunCodingPlan,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub api_key_env: String,
    pub anthropic_version: String,
    pub system_prompt: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LlmConfigStore {
    pub active_profile: String,
    pub profiles: HashMap<String, LlmConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ToolPermissionConfig {
    pub default: PermissionMode,
    pub tools: HashMap<String, PermissionMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    ClearOrExit,
    ScrollUp,
    ScrollDown,
    NavigateUp,
    NavigateDown,
    MoveLeft,
    MoveRight,
    MoveLineStart,
    MoveLineEnd,
    DeleteBackward,
    DeleteForward,
    InsertNewline,
    SubmitInput,
    ClearInput,
    OpenConfig,
    SaveConfig,
    CloseConfig,
    ConfigNextField,
    ConfigPreviousField,
    ApproveTool,
    RejectTool,
}

#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: HashMap<Action, Vec<KeyBinding>>,
    labels: HashMap<Action, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
}

#[derive(Debug, Deserialize)]
struct AppConfigFile {
    #[serde(default)]
    keybindings: KeyBindingsFile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct KeyBindingsFile {
    quit: Vec<String>,
    clear_or_exit: Vec<String>,
    scroll_up: Vec<String>,
    scroll_down: Vec<String>,
    navigate_up: Vec<String>,
    navigate_down: Vec<String>,
    move_left: Vec<String>,
    move_right: Vec<String>,
    move_line_start: Vec<String>,
    move_line_end: Vec<String>,
    delete_backward: Vec<String>,
    delete_forward: Vec<String>,
    insert_newline: Vec<String>,
    submit_input: Vec<String>,
    clear_input: Vec<String>,
    open_config: Vec<String>,
    save_config: Vec<String>,
    close_config: Vec<String>,
    config_next_field: Vec<String>,
    config_previous_field: Vec<String>,
    approve_tool: Vec<String>,
    reject_tool: Vec<String>,
}

impl KeyBindings {
    pub fn load_from_path(path: &Path) -> AppResult<Self> {
        let config = if path.exists() {
            let content = fs::read_to_string(path)?;
            toml::from_str::<AppConfigFile>(&content)?.keybindings
        } else {
            KeyBindingsFile::default()
        };

        Self::from_config(config)
    }

    pub fn matches(&self, key: &KeyEvent, action: Action) -> bool {
        self.bindings
            .get(&action)
            .into_iter()
            .flatten()
            .any(|binding| binding.matches(key))
    }

    pub fn label(&self, action: Action) -> &str {
        self.labels
            .get(&action)
            .map(String::as_str)
            .unwrap_or("unbound")
    }

    fn from_config(config: KeyBindingsFile) -> AppResult<Self> {
        let definitions = [
            (Action::Quit, config.quit),
            (Action::ClearOrExit, config.clear_or_exit),
            (Action::ScrollUp, config.scroll_up),
            (Action::ScrollDown, config.scroll_down),
            (Action::NavigateUp, config.navigate_up),
            (Action::NavigateDown, config.navigate_down),
            (Action::MoveLeft, config.move_left),
            (Action::MoveRight, config.move_right),
            (Action::MoveLineStart, config.move_line_start),
            (Action::MoveLineEnd, config.move_line_end),
            (Action::DeleteBackward, config.delete_backward),
            (Action::DeleteForward, config.delete_forward),
            (Action::InsertNewline, config.insert_newline),
            (Action::SubmitInput, config.submit_input),
            (Action::ClearInput, config.clear_input),
            (Action::OpenConfig, config.open_config),
            (Action::SaveConfig, config.save_config),
            (Action::CloseConfig, config.close_config),
            (Action::ConfigNextField, config.config_next_field),
            (Action::ConfigPreviousField, config.config_previous_field),
            (Action::ApproveTool, config.approve_tool),
            (Action::RejectTool, config.reject_tool),
        ];

        let mut bindings = HashMap::new();
        let mut labels = HashMap::new();

        for (action, raw_bindings) in definitions {
            let parsed = raw_bindings
                .iter()
                .map(|value| KeyBinding::parse(value))
                .collect::<Result<Vec<_>, _>>()?;

            bindings.insert(action, parsed);
            labels.insert(action, raw_bindings.join(" / "));
        }

        Ok(Self { bindings, labels })
    }
}

impl LlmConfigStore {
    pub fn load_from_path(path: &Path) -> AppResult<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)?;

            if let Ok(store) = toml::from_str::<Self>(&content) {
                return Ok(store.normalized());
            }

            if let Ok(config) = toml::from_str::<LlmConfig>(&content) {
                return Ok(Self::from_single(config));
            }

            return Err("failed to parse config/llm.toml as profile store or single profile".into());
        }

        Ok(Self::default())
    }

    pub fn active_profile_name(&self) -> &str {
        &self.active_profile
    }

    pub fn active_config(&self) -> AppResult<LlmConfig> {
        self.profiles
            .get(&self.active_profile)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "active profile '{}' not found in config/llm.toml",
                    self.active_profile
                )
                .into()
            })
    }

    pub fn upsert_profile(&mut self, name: impl Into<String>, config: LlmConfig) {
        self.profiles.insert(name.into(), config);
    }

    pub fn save_to_path(&self, path: &Path) -> AppResult<()> {
        let content = toml::to_string_pretty(&self.clone().normalized())?;
        fs::write(path, content)?;
        Ok(())
    }

    fn from_single(config: LlmConfig) -> Self {
        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), config);

        Self {
            active_profile: "default".to_string(),
            profiles,
        }
    }

    fn normalized(mut self) -> Self {
        if self.profiles.is_empty() {
            self.profiles
                .insert("default".to_string(), LlmConfig::default());
        }

        if self.active_profile.trim().is_empty() || !self.profiles.contains_key(&self.active_profile)
        {
            let mut names = self.profiles.keys().cloned().collect::<Vec<_>>();
            names.sort();
            self.active_profile = names
                .into_iter()
                .next()
                .unwrap_or_else(|| "default".to_string());
        }

        self
    }
}

impl ToolPermissionConfig {
    pub fn load_from_path(path: &Path) -> AppResult<Self> {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            return Ok(toml::from_str::<Self>(&content)?);
        }

        Ok(Self::default())
    }

    pub fn mode_for(&self, tool_name: &str) -> PermissionMode {
        self.tools.get(tool_name).copied().unwrap_or(self.default)
    }
}

impl LlmConfig {
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var(&self.api_key_env).ok())
    }
}

impl KeyBinding {
    fn parse(value: &str) -> AppResult<Self> {
        if value.trim().eq_ignore_ascii_case("shift+tab") {
            return Ok(Self {
                code: KeyCode::BackTab,
                modifiers: KeyModifiers::empty(),
            });
        }

        let parts: Vec<_> = value
            .split('+')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect();

        if parts.is_empty() {
            return Err("empty keybinding".into());
        }

        let mut modifiers = KeyModifiers::empty();
        for modifier in &parts[..parts.len().saturating_sub(1)] {
            match modifier.as_str() {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "alt" => modifiers |= KeyModifiers::ALT,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                _ => return Err(format!("unsupported modifier: {modifier}").into()),
            }
        }

        let code = parse_key_code(parts.last().expect("parts is not empty"))?;
        Ok(Self { code, modifiers })
    }

    fn matches(&self, event: &KeyEvent) -> bool {
        normalize_modifiers(event.modifiers) == self.modifiers
            && key_code_matches(&self.code, &event.code)
    }
}

impl Default for KeyBindingsFile {
    fn default() -> Self {
        Self {
            quit: vec!["q".to_string()],
            clear_or_exit: vec!["ctrl+c".to_string()],
            scroll_up: vec!["pageup".to_string()],
            scroll_down: vec!["pagedown".to_string()],
            navigate_up: vec!["up".to_string()],
            navigate_down: vec!["down".to_string()],
            move_left: vec!["left".to_string()],
            move_right: vec!["right".to_string()],
            move_line_start: vec!["home".to_string()],
            move_line_end: vec!["end".to_string()],
            delete_backward: vec!["backspace".to_string()],
            delete_forward: vec!["delete".to_string()],
            insert_newline: vec!["alt+enter".to_string()],
            submit_input: vec!["enter".to_string()],
            clear_input: vec!["esc".to_string()],
            open_config: vec!["f2".to_string()],
            save_config: vec!["ctrl+s".to_string()],
            close_config: vec!["esc".to_string()],
            config_next_field: vec!["tab".to_string()],
            config_previous_field: vec!["shift+tab".to_string()],
            approve_tool: vec!["y".to_string(), "enter".to_string()],
            reject_tool: vec!["n".to_string(), "esc".to_string()],
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4.1-mini".to_string(),
            api_key: None,
            api_key_env: "OPENAI_API_KEY".to_string(),
            anthropic_version: "2023-06-01".to_string(),
            system_prompt: "You are a helpful private CLI assistant.".to_string(),
            temperature: 0.2,
            max_tokens: 2048,
            timeout_seconds: 120,
        }
    }
}

impl Default for LlmConfigStore {
    fn default() -> Self {
        Self::from_single(LlmConfig::default())
    }
}

impl Default for ToolPermissionConfig {
    fn default() -> Self {
        let mut tools = HashMap::new();
        tools.insert("run_command".to_string(), PermissionMode::Ask);
        tools.insert("write_file".to_string(), PermissionMode::Ask);
        tools.insert("apply_patch".to_string(), PermissionMode::Ask);
        tools.insert("delete_path".to_string(), PermissionMode::Ask);
        tools.insert("move_path".to_string(), PermissionMode::Ask);
        tools.insert("make_directory".to_string(), PermissionMode::Ask);

        Self {
            default: PermissionMode::Allow,
            tools,
        }
    }
}

fn parse_key_code(value: &str) -> AppResult<KeyCode> {
    let code = match value {
        "backtab" => KeyCode::BackTab,
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "tab" => KeyCode::Tab,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        "space" => KeyCode::Char(' '),
        other if other.chars().count() == 1 => {
            KeyCode::Char(other.chars().next().unwrap_or_default())
        }
        _ => return Err(format!("unsupported key code: {value}").into()),
    };

    Ok(code)
}

fn normalize_modifiers(modifiers: KeyModifiers) -> KeyModifiers {
    let mut normalized = KeyModifiers::empty();

    if modifiers.contains(KeyModifiers::SHIFT) {
        normalized |= KeyModifiers::SHIFT;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        normalized |= KeyModifiers::CONTROL;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        normalized |= KeyModifiers::ALT;
    }

    normalized
}

fn key_code_matches(expected: &KeyCode, actual: &KeyCode) -> bool {
    match (expected, actual) {
        (KeyCode::Char(expected), KeyCode::Char(actual)) => {
            expected.eq_ignore_ascii_case(actual)
        }
        _ => expected == actual,
    }
}
