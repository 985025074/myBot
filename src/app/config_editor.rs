use std::{collections::HashMap, path::{Path, PathBuf}};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    app::{AppResult, InputEditor},
    config::{Action, KeyBindings, LlmConfig, LlmConfigStore, ProviderKind},
};

#[derive(Debug)]
pub struct ConfigEditor {
    profiles: HashMap<String, LlmConfig>,
    current_profile: String,
    fields: Vec<ConfigField>,
    selected: usize,
    editor: InputEditor,
    path: PathBuf,
    status: String,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub enum ConfigEvent {
    Close,
    Saved(LlmConfigStore),
}

#[derive(Debug, Clone)]
struct ConfigField {
    id: ConfigFieldId,
    label: &'static str,
    help: &'static str,
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFieldId {
    Profile,
    Provider,
    BaseUrl,
    Model,
    ApiKeyEnv,
    ApiKey,
    AnthropicVersion,
    SystemPrompt,
    Temperature,
    MaxTokens,
    TimeoutSeconds,
}

impl ConfigEditor {
    pub fn new(store: &LlmConfigStore, path: impl Into<PathBuf>) -> Self {
        let current_profile = store.active_profile_name().to_string();
        let current_config = store.active_config().unwrap_or_default();
        let fields = fields_from_config(&current_profile, &current_config);
        let mut editor = InputEditor::new();
        if let Some(field) = fields.first() {
            editor.set_text(field.value.clone());
        }

        Self {
            profiles: store.profiles.clone(),
            current_profile,
            fields,
            selected: 0,
            editor,
            path: path.into(),
            status: format!(
                "当前 profile：{}。编辑后按 Ctrl+S 保存，Esc 关闭。",
                store.active_profile_name()
            ),
            dirty: false,
        }
    }

    pub fn sync_viewport(&mut self, width: u16, height: u16) {
        self.editor.set_viewport(width as usize, height as usize);
    }

    pub fn visible_lines(&self) -> Vec<String> {
        self.editor.visible_lines()
    }

    pub fn cursor_screen_position(&self) -> (u16, u16) {
        self.editor.cursor_screen_position()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn selected_label(&self) -> &str {
        self.fields[self.selected].label
    }

    pub fn selected_help(&self) -> &str {
        self.fields[self.selected].help
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn field_lines(&self) -> Vec<String> {
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let prefix = if index == self.selected { ">" } else { " " };
                format!("{prefix} {}: {}", field.label, summarize(&field.value))
            })
            .collect()
    }

    pub fn handle_key(
        &mut self,
        key: &KeyEvent,
        keybindings: &KeyBindings,
    ) -> Option<ConfigEvent> {
        if keybindings.matches(key, Action::SaveConfig) {
            match self.build_store() {
                Ok(store) => match store.save_to_path(&self.path) {
                    Ok(()) => {
                        self.status = format!(
                            "已保存到 {}，激活 profile：{}。可用 profiles：{}",
                            display_path(&self.path),
                            self.current_profile,
                            self.profile_names().join(", ")
                        );
                        self.dirty = false;
                        return Some(ConfigEvent::Saved(store));
                    }
                    Err(error) => {
                        self.status = format!("保存失败：{error}");
                        return None;
                    }
                },
                Err(error) => {
                    self.status = format!("配置无效：{error}");
                    return None;
                }
            }
        }

        if keybindings.matches(key, Action::CloseConfig) {
            self.commit_current_field();
            return Some(ConfigEvent::Close);
        }

        if keybindings.matches(key, Action::ConfigNextField)
            || keybindings.matches(key, Action::SubmitInput)
        {
            self.select_next_field();
            return None;
        }

        if keybindings.matches(key, Action::ConfigPreviousField) {
            self.select_previous_field();
            return None;
        }

        if keybindings.matches(key, Action::NavigateUp) {
            self.editor.move_up();
            return None;
        }

        if keybindings.matches(key, Action::NavigateDown) {
            self.editor.move_down();
            return None;
        }

        if keybindings.matches(key, Action::MoveLeft) {
            self.editor.move_left();
            return None;
        }

        if keybindings.matches(key, Action::MoveRight) {
            self.editor.move_right();
            return None;
        }

        if keybindings.matches(key, Action::MoveLineStart) {
            self.editor.move_to_line_start();
            return None;
        }

        if keybindings.matches(key, Action::MoveLineEnd) {
            self.editor.move_to_line_end();
            return None;
        }

        if keybindings.matches(key, Action::DeleteBackward) {
            self.editor.delete_before_cursor();
            self.dirty = true;
            return None;
        }

        if keybindings.matches(key, Action::DeleteForward) {
            self.editor.delete_at_cursor();
            self.dirty = true;
            return None;
        }

        if keybindings.matches(key, Action::InsertNewline) {
            self.editor.insert_newline();
            self.dirty = true;
            return None;
        }

        if keybindings.matches(key, Action::ClearInput) {
            self.editor.clear();
            self.dirty = true;
            return None;
        }

        if let KeyCode::Char(c) = key.code
            && !key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            self.editor.insert_char(c);
            self.dirty = true;
        }

        None
    }

    fn commit_current_field(&mut self) {
        self.fields[self.selected].value = self.editor.text().to_string();
    }

    fn select_next_field(&mut self) {
        if self.commit_and_maybe_switch_profile().is_err() {
            return;
        }

        self.selected = (self.selected + 1) % self.fields.len();
        self.editor
            .set_text(self.fields[self.selected].value.clone());
    }

    fn select_previous_field(&mut self) {
        if self.commit_and_maybe_switch_profile().is_err() {
            return;
        }

        self.selected = if self.selected == 0 {
            self.fields.len() - 1
        } else {
            self.selected - 1
        };
        self.editor
            .set_text(self.fields[self.selected].value.clone());
    }

    fn commit_and_maybe_switch_profile(&mut self) -> AppResult<()> {
        self.commit_current_field();
        if self.fields[self.selected].id == ConfigFieldId::Profile {
            self.switch_profile()?;
        }
        Ok(())
    }

    fn switch_profile(&mut self) -> AppResult<()> {
        let target_profile = self.profile_field_value()?;
        let current_config = self.build_current_config()?;
        self.profiles
            .insert(self.current_profile.clone(), current_config.clone());

        let target_exists = self.profiles.contains_key(&target_profile);
        let target_config = self
            .profiles
            .get(&target_profile)
            .cloned()
            .unwrap_or(current_config);

        self.profiles
            .insert(target_profile.clone(), target_config.clone());
        self.current_profile = target_profile.clone();
        self.fields = fields_from_config(&self.current_profile, &target_config);
        self.selected = self
            .fields
            .iter()
            .position(|field| field.id == ConfigFieldId::Provider)
            .unwrap_or(0);
        self.editor
            .set_text(self.fields[self.selected].value.clone());
        self.status = if target_exists {
            format!(
                "已切换到 profile：{}。可用 profiles：{}",
                self.current_profile,
                self.profile_names().join(", ")
            )
        } else {
            self.dirty = true;
            format!(
                "已创建新 profile：{}。保存后写入文件。",
                self.current_profile
            )
        };
        Ok(())
    }

    fn build_store(&mut self) -> AppResult<LlmConfigStore> {
        self.commit_current_field();

        let target_profile = self.profile_field_value()?;
        if target_profile != self.current_profile {
            self.switch_profile()?;
        }

        let current_config = self.build_current_config()?;
        self.profiles
            .insert(self.current_profile.clone(), current_config);

        let mut store = LlmConfigStore {
            active_profile: self.current_profile.clone(),
            profiles: self.profiles.clone(),
        };
        if !store.profiles.contains_key(&store.active_profile) {
            store.upsert_profile(store.active_profile.clone(), LlmConfig::default());
        }
        Ok(store)
    }

    fn build_current_config(&self) -> AppResult<LlmConfig> {
        let provider = parse_provider(self.value(ConfigFieldId::Provider)?)?;
        let temperature = self
            .value(ConfigFieldId::Temperature)?
            .trim()
            .parse::<f32>()?;
        let max_tokens = self
            .value(ConfigFieldId::MaxTokens)?
            .trim()
            .parse::<u32>()?;
        let timeout_seconds = self
            .value(ConfigFieldId::TimeoutSeconds)?
            .trim()
            .parse::<u64>()?;
        let api_key = normalize_optional(self.value(ConfigFieldId::ApiKey)?);

        Ok(LlmConfig {
            provider,
            base_url: self.value(ConfigFieldId::BaseUrl)?.to_string(),
            model: self.value(ConfigFieldId::Model)?.to_string(),
            api_key,
            api_key_env: self.value(ConfigFieldId::ApiKeyEnv)?.to_string(),
            anthropic_version: self.value(ConfigFieldId::AnthropicVersion)?.to_string(),
            system_prompt: self.value(ConfigFieldId::SystemPrompt)?.to_string(),
            temperature,
            max_tokens,
            timeout_seconds,
        })
    }

    fn value(&self, id: ConfigFieldId) -> AppResult<&str> {
        self.fields
            .iter()
            .find(|field| field.id == id)
            .map(|field| field.value.as_str())
            .ok_or_else(|| format!("missing config field: {id:?}").into())
    }

    fn profile_field_value(&self) -> AppResult<String> {
        let profile = self.value(ConfigFieldId::Profile)?.trim();
        if profile.is_empty() {
            return Err("profile 名称不能为空".into());
        }
        Ok(profile.to_string())
    }

    fn profile_names(&self) -> Vec<String> {
        let mut names = self.profiles.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }
}

fn fields_from_config(profile_name: &str, config: &LlmConfig) -> Vec<ConfigField> {
    vec![
        ConfigField {
            id: ConfigFieldId::Profile,
            label: "profile",
            help: "配置名称。输入已有名称可切换；输入新名称可基于当前配置创建一套新 profile。",
            value: profile_name.to_string(),
        },
        ConfigField {
            id: ConfigFieldId::Provider,
            label: "provider",
            help: "可选 open-ai-compatible、anthropic、aliyun-coding-plan。阿里云百炼 Coding Plan 走 Anthropic 兼容接口。",
            value: provider_to_string(config.provider).to_string(),
        },
        ConfigField {
            id: ConfigFieldId::BaseUrl,
            label: "base_url",
            help: "Provider API 根地址。Aliyun Coding Plan 建议使用 https://coding.dashscope.aliyuncs.com/apps/anthropic/v1 。",
            value: config.base_url.clone(),
        },
        ConfigField {
            id: ConfigFieldId::Model,
            label: "model",
            help: "模型标识，例如 gpt-4.1-mini、claude-3-7-sonnet-latest、qwen3-coder-plus。",
            value: config.model.clone(),
        },
        ConfigField {
            id: ConfigFieldId::ApiKeyEnv,
            label: "api_key_env",
            help: "用于读取 API key 的环境变量名。Aliyun Coding Plan 可用 BAILIAN_CODING_PLAN_API_KEY。",
            value: config.api_key_env.clone(),
        },
        ConfigField {
            id: ConfigFieldId::ApiKey,
            label: "api_key",
            help: "可直接写入 API key；留空则优先走环境变量。",
            value: config.api_key.clone().unwrap_or_default(),
        },
        ConfigField {
            id: ConfigFieldId::AnthropicVersion,
            label: "anthropic_version",
            help: "Anthropic Messages API 使用的版本 header。",
            value: config.anthropic_version.clone(),
        },
        ConfigField {
            id: ConfigFieldId::SystemPrompt,
            label: "system_prompt",
            help: "系统提示词。支持多行，使用 Alt+Enter 换行。",
            value: config.system_prompt.clone(),
        },
        ConfigField {
            id: ConfigFieldId::Temperature,
            label: "temperature",
            help: "采样温度，通常在 0.0 到 1.0 之间。",
            value: config.temperature.to_string(),
        },
        ConfigField {
            id: ConfigFieldId::MaxTokens,
            label: "max_tokens",
            help: "单次回复的最大输出 token。",
            value: config.max_tokens.to_string(),
        },
        ConfigField {
            id: ConfigFieldId::TimeoutSeconds,
            label: "timeout_seconds",
            help: "HTTP 请求超时时间，单位秒。",
            value: config.timeout_seconds.to_string(),
        },
    ]
}

fn provider_to_string(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::OpenAiCompatible => "open-ai-compatible",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::AliyunCodingPlan => "aliyun-coding-plan",
    }
}

fn parse_provider(value: &str) -> AppResult<ProviderKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "open-ai-compatible" | "openai-compatible" | "openai" => {
            Ok(ProviderKind::OpenAiCompatible)
        }
        "anthropic" => Ok(ProviderKind::Anthropic),
        "aliyun-coding-plan" | "bailian-coding-plan" | "dashscope-coding-plan" => {
            Ok(ProviderKind::AliyunCodingPlan)
        }
        other => Err(format!("unsupported provider: {other}").into()),
    }
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn summarize(value: &str) -> String {
    let compact = value.replace('\n', " ⏎ ");
    let chars = compact.chars().collect::<Vec<_>>();
    if chars.len() <= 28 {
        compact
    } else {
        chars[..28].iter().collect::<String>() + "…"
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
