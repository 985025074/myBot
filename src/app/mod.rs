mod commands;
mod config_editor;
mod input;
pub mod session;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender, TryRecvError},
};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::Value;

use crate::{
    agent::{
        AgentExecutor, AgentThreadMessage, ToolApprovalDecision,
        ToolApprovalRequest,
    },
    config::{Action, KeyBindings, LlmConfigStore, ToolPermissionConfig},
    llm::{ChatMessage, LlmClient},
    setup::RuntimeScope,
    skills::SkillStore,
    tools::{
        CustomToolStore, ToolContext, ToolRegistry, WorkspaceUndoSnapshot,
        apply_workspace_undo_snapshot,
    },
};
use commands::{
    SlashCommand, all as all_slash_commands,
    autocomplete_selected as autocomplete_slash_command_selected, find as find_slash_command,
    suggestions as slash_command_suggestions,
};
use session::{SessionData, SessionStore, SessionSummary};

pub use config_editor::{ConfigEditor, ConfigEvent};
pub use input::InputEditor;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) const TOOL_LOG_MARKER_PREFIX: &str = "__tool_log__:";

#[derive(Debug, Clone, Copy, Default)]
struct Viewport {
    width: u16,
    height: u16,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ToolLogSection {
    pub title: String,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ActiveStreamPreview {
    pub step: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct SessionPickerState {
    pub sessions: Vec<SessionSummary>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct SkillPickerEntry {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct SkillPickerState {
    pub skills: Vec<SkillPickerEntry>,
    pub selected: usize,
}

#[derive(Debug)]
pub struct SessionRenameState {
    pub session_id: String,
    pub original_title: String,
    pub editor: InputEditor,
}

#[derive(Debug, Clone)]
struct UndoSnapshot {
    messages: Vec<String>,
    chat_history: Vec<ChatMessage>,
    tool_logs: Vec<ToolLogSection>,
    workspace_undo: Vec<WorkspaceUndoSnapshot>,
    session_title: String,
    show_thinking: bool,
    show_tool_details: bool,
}

#[derive(Debug)]
pub struct App {
    pub editor: InputEditor,
    pub messages: Vec<String>,
    pub conversation_scroll: u16,
    keybindings: KeyBindings,
    llm: LlmClient,
    llm_profiles: LlmConfigStore,
    llm_config_path: PathBuf,
    tool_permissions: ToolPermissionConfig,
    tools: ToolRegistry,
    skills: Arc<SkillStore>,
    custom_tools: Arc<CustomToolStore>,
    runtime_scope: RuntimeScope,
    runtime_root: PathBuf,
    tool_context: ToolContext,
    session_store: SessionStore,
    session_id: String,
    session_title: String,
    session_created_at: u64,
    session_dirty: bool,
    session_allowed_tools: Arc<Mutex<HashSet<String>>>,
    session_denied_tools: Arc<Mutex<HashSet<String>>>,
    chat_history: Vec<ChatMessage>,
    tool_logs: Vec<ToolLogSection>,
    active_tool_events: Vec<String>,
    active_stream_preview: Option<ActiveStreamPreview>,
    skill_picker: Option<SkillPickerState>,
    session_picker: Option<SessionPickerState>,
    session_rename: Option<SessionRenameState>,
    undo_stack: Vec<UndoSnapshot>,
    command_suggestion_index: usize,
    show_thinking: bool,
    show_tool_details: bool,
    pending_response: Option<Receiver<AgentThreadMessage>>,
    pending_approval_tx: Option<Sender<ToolApprovalDecision>>,
    pending_tool_approval: Option<ToolApprovalRequest>,
    config_editor: Option<ConfigEditor>,
    conversation_viewport: Viewport,
    ctrl_c_armed: bool,
}

impl App {
    pub fn default_welcome_messages(llm: &LlmClient) -> Vec<String> {
        vec![
            "assistant> 欢迎使用 mybot TUI 骨架。".to_string(),
            format!(
                "assistant> 已接入 provider 抽象，当前 provider：{}，模型：{}。",
                llm.provider_name(),
                llm.model_name()
            ),
            "assistant> 已引入模块化工具系统。输入 /help 查看命令，/tools 查看工具，/sessions 查看会话。"
                .to_string(),
            "assistant> 现在支持历史输入、多行输入、输入区自动换行和聊天区滚动。"
                .to_string(),
        ]
    }

    pub fn new(
        keybindings: KeyBindings,
        llm: LlmClient,
        llm_profiles: LlmConfigStore,
        llm_config_path: PathBuf,
        tool_permissions: ToolPermissionConfig,
        tools: ToolRegistry,
        skills: Arc<SkillStore>,
        custom_tools: Arc<CustomToolStore>,
        runtime_scope: RuntimeScope,
        runtime_root: PathBuf,
        tool_context: ToolContext,
        session_store: SessionStore,
        initial_session: SessionData,
    ) -> AppResult<Self> {
        Ok(Self {
            editor: InputEditor::new(),
            messages: initial_session.messages.clone(),
            conversation_scroll: 0,
            keybindings,
            llm,
            llm_profiles,
            llm_config_path,
            tool_permissions,
            tools,
            skills,
            custom_tools,
            runtime_scope,
            runtime_root,
            tool_context,
            session_store,
            session_id: initial_session.id.clone(),
            session_title: initial_session.title.clone(),
            session_created_at: initial_session.created_at,
            session_dirty: false,
            session_allowed_tools: Arc::new(Mutex::new(HashSet::new())),
            session_denied_tools: Arc::new(Mutex::new(HashSet::new())),
            chat_history: initial_session.chat_history,
            tool_logs: initial_session.tool_logs,
            active_tool_events: Vec::new(),
            active_stream_preview: None,
            skill_picker: None,
            session_picker: None,
            session_rename: None,
            undo_stack: Vec::new(),
            command_suggestion_index: 0,
            show_thinking: initial_session.show_thinking,
            show_tool_details: initial_session.show_tool_details,
            pending_response: None,
            pending_approval_tx: None,
            pending_tool_approval: None,
            config_editor: None,
            conversation_viewport: Viewport::default(),
            ctrl_c_armed: false,
        })
    }

    pub fn tick(&mut self) {
        let Some(receiver) = self.pending_response.take() else {
            self.persist_session_if_dirty();
            return;
        };

        match receiver.try_recv() {
            Ok(message) => match message {
                AgentThreadMessage::StreamingChunk { step, chunk } => {
                    let preview = self
                        .active_stream_preview
                        .get_or_insert_with(ActiveStreamPreview::default);
                    if preview.step != step {
                        preview.step = step;
                        preview.text.clear();
                    }
                    preview.text.push_str(&chunk);
                    self.pending_response = Some(receiver);
                    self.scroll_conversation_to_bottom();
                }
                AgentThreadMessage::StreamingFinished { step } => {
                    if self
                        .active_stream_preview
                        .as_ref()
                        .map(|preview| preview.step == step)
                        .unwrap_or(false)
                    {
                        self.active_stream_preview = None;
                    }
                    self.pending_response = Some(receiver);
                }
                AgentThreadMessage::ToolEvent(event) => {
                    if event.starts_with("skill> ") {
                        self.messages.push(event);
                        self.mark_session_dirty();
                    } else {
                        self.active_tool_events.push(event);
                    }
                    self.pending_response = Some(receiver);
                    self.scroll_conversation_to_bottom();
                }
                AgentThreadMessage::WorkspaceUndo(snapshot) => {
                    self.record_workspace_undo(snapshot);
                    self.pending_response = Some(receiver);
                }
                AgentThreadMessage::ApprovalRequired(request) => {
                    self.active_stream_preview = None;
                    self.pending_tool_approval = Some(request);
                    self.pending_response = Some(receiver);
                }
                AgentThreadMessage::Finished(result) => match result {
                    Ok(result) => {
                        self.active_stream_preview = None;
                        self.pending_approval_tx = None;
                        self.pending_tool_approval = None;
                        if !self.active_tool_events.is_empty() {
                            let count = self.active_tool_events.len();
                            let events = std::mem::take(&mut self.active_tool_events);
                            self.push_tool_log(format!("工具执行详情（{} 条）", count), events);
                        } else {
                            let non_skill_events = result
                                .events
                                .iter()
                                .cloned()
                                .into_iter()
                                .filter(|event| !event.starts_with("skill> "))
                                .collect::<Vec<_>>();
                            if !non_skill_events.is_empty() {
                                self.push_tool_log(
                                    format!("工具执行详情（{} 条）", non_skill_events.len()),
                                    non_skill_events,
                                );
                            }
                        }
                        self.chat_history.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: result.final_reply.clone(),
                        });
                        self.messages
                            .push(format!("assistant> {}", result.final_reply));
                        self.mark_session_dirty();
                        self.scroll_conversation_to_bottom();
                    }
                    Err(error) => {
                        self.active_stream_preview = None;
                        self.pending_approval_tx = None;
                        self.pending_tool_approval = None;
                        if !self.active_tool_events.is_empty() {
                            let count = self.active_tool_events.len();
                            let events = std::mem::take(&mut self.active_tool_events);
                            self.push_tool_log(format!("工具执行详情（{} 条）", count), events);
                        }
                        self.messages.push(format!("assistant> 请求失败：{error}"));
                        self.mark_session_dirty();
                        self.scroll_conversation_to_bottom();
                    }
                },
            },
            Err(TryRecvError::Empty) => {
                self.pending_response = Some(receiver);
            }
            Err(TryRecvError::Disconnected) => {
                self.active_stream_preview = None;
                self.pending_approval_tx = None;
                self.pending_tool_approval = None;
                if !self.active_tool_events.is_empty() {
                    let count = self.active_tool_events.len();
                    let events = std::mem::take(&mut self.active_tool_events);
                    self.push_tool_log(format!("工具执行详情（{} 条）", count), events);
                }
                self.messages
                    .push("assistant> 请求通道已断开，请重试。".to_string());
                self.mark_session_dirty();
                self.scroll_conversation_to_bottom();
            }
        }

        self.persist_session_if_dirty();
    }

    pub fn ctrl_c_armed(&self) -> bool {
        self.ctrl_c_armed
    }

    pub fn is_waiting_for_reply(&self) -> bool {
        self.pending_response.is_some()
    }

    pub fn is_config_open(&self) -> bool {
        self.config_editor.is_some()
    }

    pub fn is_session_picker_open(&self) -> bool {
        self.session_picker.is_some()
    }

    pub fn is_skill_picker_open(&self) -> bool {
        self.skill_picker.is_some()
    }

    pub fn skill_picker(&self) -> Option<&SkillPickerState> {
        self.skill_picker.as_ref()
    }

    pub fn session_picker(&self) -> Option<&SessionPickerState> {
        self.session_picker.as_ref()
    }

    pub fn is_session_rename_open(&self) -> bool {
        self.session_rename.is_some()
    }

    pub fn session_rename(&self) -> Option<&SessionRenameState> {
        self.session_rename.as_ref()
    }

    pub fn has_pending_tool_approval(&self) -> bool {
        self.pending_tool_approval.is_some()
    }

    pub fn pending_tool_approval(&self) -> Option<&ToolApprovalRequest> {
        self.pending_tool_approval.as_ref()
    }

    pub fn config_editor(&self) -> Option<&ConfigEditor> {
        self.config_editor.as_ref()
    }

    pub fn model_name(&self) -> &str {
        self.llm.model_name()
    }

    pub fn profile_name(&self) -> &str {
        self.llm_profiles.active_profile_name()
    }

    pub fn provider_name(&self) -> &str {
        self.llm.provider_name()
    }

    pub fn tool_count(&self) -> usize {
        self.tools.definitions().len()
    }

    pub fn session_summary_label(&self) -> String {
        format!("{} ({})", self.session_title, self.short_session_id())
    }

    pub fn current_session_id(&self) -> &str {
        &self.session_id
    }

    pub fn runtime_scope_label(&self) -> &'static str {
        self.runtime_scope.as_str()
    }

    pub fn show_thinking(&self) -> bool {
        self.show_thinking
    }

    pub fn show_tool_details(&self) -> bool {
        self.show_tool_details
    }

    pub fn active_stream_preview(&self) -> Option<&ActiveStreamPreview> {
        self.active_stream_preview.as_ref()
    }

    pub fn tool_logs(&self) -> &[ToolLogSection] {
        &self.tool_logs
    }

    pub fn active_tool_events(&self) -> &[String] {
        &self.active_tool_events
    }

    pub fn key_label(&self, action: Action) -> &str {
        self.keybindings.label(action)
    }

    pub fn command_hint_lines(&self) -> Vec<String> {
        let suggestions = self.command_suggestions();
        if suggestions.is_empty() {
            return Vec::new();
        }

        let token = self
            .editor
            .text()
            .trim_start()
            .split_whitespace()
            .next()
            .unwrap_or_default();

        if let Some(command) = find_slash_command(token) {
            return vec![
                format!("命令提示: {} · {}", command.usage, command.summary),
                format!("{} 可自动补全命令", self.key_label(Action::AutocompleteCommand)),
            ];
        }

        vec![
            format!(
                "命令提示: {}",
                suggestions
                    .into_iter()
                    .take(4)
                    .map(|command| format!("{}({})", command.name, command.summary))
                    .collect::<Vec<_>>()
                    .join("  ·  ")
            ),
            format!("{} 补全到最接近命令", self.key_label(Action::AutocompleteCommand)),
        ]
    }

    pub fn command_suggestions(&self) -> Vec<&'static SlashCommand> {
        slash_command_suggestions(self.editor.text())
    }

    pub fn selected_command_suggestion_index(&self) -> Option<usize> {
        let suggestions = self.command_suggestions();
        if suggestions.is_empty() {
            None
        } else {
            Some(self.command_suggestion_index.min(suggestions.len().saturating_sub(1)))
        }
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        let Event::Key(key) = event else {
            return false;
        };

        if key.kind != KeyEventKind::Press {
            return false;
        }

        if self.config_editor.is_some() {
            return self.handle_config_event(key);
        }

        if self.session_rename.is_some() {
            return self.handle_session_rename_event(key);
        }

        if self.skill_picker.is_some() {
            return self.handle_skill_picker_event(key);
        }

        if self.session_picker.is_some() {
            return self.handle_session_picker_event(key);
        }

        if self.pending_tool_approval.is_some() {
            return self.handle_tool_approval_event(key);
        }

        let modifiers = key.modifiers;

        if self.keybindings.matches(&key, Action::OpenConfig) {
            self.open_config_editor();
            return false;
        }

        if self.keybindings.matches(&key, Action::ToggleThinking) {
            self.show_thinking = !self.show_thinking;
            self.mark_session_dirty();
            self.scroll_conversation_to_bottom();
            return false;
        }

        if self.keybindings.matches(&key, Action::ToggleToolDetails) {
            self.show_tool_details = !self.show_tool_details;
            self.mark_session_dirty();
            self.scroll_conversation_to_bottom();
            return false;
        }

        if self.keybindings.matches(&key, Action::AutocompleteCommand) {
            self.autocomplete_command();
            return false;
        }

        if self.keybindings.matches(&key, Action::ClearOrExit) {
            return self.handle_ctrl_c();
        }

        self.ctrl_c_armed = false;

        if self.keybindings.matches(&key, Action::Quit) && self.editor.is_empty() {
            return true;
        }

        if self.keybindings.matches(&key, Action::ScrollUp) {
            self.scroll_conversation_up(self.page_scroll_amount());
            return false;
        }

        if self.keybindings.matches(&key, Action::ScrollDown) {
            self.scroll_conversation_down(self.page_scroll_amount());
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateUp) {
            if self.try_select_previous_command_suggestion() {
                return false;
            }
            if self.editor.is_cursor_on_first_line() {
                self.editor.use_older_history();
                self.reset_command_suggestion_selection();
            } else {
                self.editor.move_up();
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateDown) {
            if self.try_select_next_command_suggestion() {
                return false;
            }
            if self.editor.is_cursor_on_last_line() {
                self.editor.use_newer_history();
                self.reset_command_suggestion_selection();
            } else {
                self.editor.move_down();
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLeft) {
            self.editor.move_left();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveRight) {
            self.editor.move_right();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLineStart) {
            self.editor.move_to_line_start();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLineEnd) {
            self.editor.move_to_line_end();
            return false;
        }

        if self.keybindings.matches(&key, Action::DeleteBackward) {
            self.editor.delete_before_cursor();
            self.reset_command_suggestion_selection();
            return false;
        }

        if self.keybindings.matches(&key, Action::DeleteForward) {
            self.editor.delete_at_cursor();
            self.reset_command_suggestion_selection();
            return false;
        }

        if self.keybindings.matches(&key, Action::InsertNewline) {
            self.editor.insert_newline();
            self.reset_command_suggestion_selection();
            return false;
        }

        if self.keybindings.matches(&key, Action::SubmitInput) {
            self.submit_input();
            return false;
        }

        if self.keybindings.matches(&key, Action::ClearInput) {
            self.editor.clear();
            self.reset_command_suggestion_selection();
            return false;
        }

        match key.code {
            KeyCode::Char(c)
                if !modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                self.editor.insert_char(c);
                self.reset_command_suggestion_selection();
                false
            }
            _ => false,
        }
    }

    pub fn sync_viewports(
        &mut self,
        conversation_width: u16,
        conversation_height: u16,
        input_width: u16,
        input_height: u16,
    ) {
        self.conversation_viewport = Viewport {
            width: conversation_width,
            height: conversation_height,
        };
        self.editor
            .set_viewport(input_width as usize, input_height as usize);
        self.clamp_conversation_scroll();
    }

    pub fn sync_config_viewport(&mut self, width: u16, height: u16) {
        if let Some(editor) = self.config_editor.as_mut() {
            editor.sync_viewport(width, height);
        }
    }

    pub fn sync_session_rename_viewport(&mut self, width: u16, height: u16) {
        if let Some(rename) = self.session_rename.as_mut() {
            rename.editor.set_viewport(width as usize, height as usize);
        }
    }

    pub fn max_conversation_scroll(&self) -> u16 {
        let width = self.conversation_viewport.width as usize;
        let height = self.conversation_viewport.height as usize;

        if width == 0 || height == 0 {
            return 0;
        }

        let total_lines = self.total_conversation_lines(width);
        total_lines.saturating_sub(height) as u16
    }

    fn submit_input(&mut self) {
        if self.pending_response.is_some() {
            self.messages
                .push("assistant> 还有一个请求在处理中，请等待当前回复完成。".to_string());
            self.scroll_conversation_to_bottom();
            return;
        }

        let Some(submitted) = self.editor.submit() else {
            return;
        };
        self.command_suggestion_index = 0;
        if !submitted.trim_start().starts_with("/undo") {
            self.push_undo_snapshot();
        }

        if self.try_handle_local_command(&submitted) {
            self.scroll_conversation_to_bottom();
            return;
        }

        self.chat_history.push(ChatMessage {
            role: "user".to_string(),
            content: submitted.clone(),
        });
        self.update_session_title_from_input(&submitted);
        self.active_tool_events.clear();
        self.active_stream_preview = None;
        self.messages.push(format!("you> {submitted}"));
        self.mark_session_dirty();
        let _ = self.save_current_session();
        self.scroll_conversation_to_bottom();

        let agent = AgentExecutor::new(
            self.llm.clone(),
            self.tools.clone(),
            self.tool_context.clone(),
        );
        let history = self.chat_history.clone();
        let permissions = self.tool_permissions.clone();
        let session_allowed_tools = self.session_allowed_tools.clone();
        let session_denied_tools = self.session_denied_tools.clone();
        let (tx, rx) = mpsc::channel();
        let (decision_tx, decision_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = agent.run(
                &history,
                |descriptor| {
                    if session_denied_tools
                        .lock()
                        .map(|denied| denied.contains(&descriptor.tool))
                        .unwrap_or(false)
                    {
                        crate::config::PermissionMode::Deny
                    } else if session_allowed_tools
                        .lock()
                        .map(|allowed| allowed.contains(&descriptor.tool))
                        .unwrap_or(false)
                    {
                        crate::config::PermissionMode::Allow
                    } else {
                        permissions.mode_for(descriptor)
                    }
                },
                tx.clone(),
                decision_rx,
            );
            let _ = tx.send(AgentThreadMessage::Finished(result));
        });
        self.pending_response = Some(rx);
        self.pending_approval_tx = Some(decision_tx);
    }

    fn try_handle_local_command(&mut self, submitted: &str) -> bool {
        let trimmed = submitted.trim();

        let command_name = trimmed
            .split_whitespace()
            .next()
            .unwrap_or_default();

        match command_name {
            "/help" | "/commands" => {
                self.messages.push(format!("you> {trimmed}"));
                self.messages.push(format!(
                    "assistant> 当前支持的 /命令:\n{}",
                    all_slash_commands()
                        .iter()
                        .map(|command| format!("- {}: {}", command.usage, command.summary))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
                self.mark_session_dirty();
                return true;
            }
            "/config" => {
                self.messages.push("you> /config".to_string());
                self.open_config_editor();
                self.mark_session_dirty();
                return true;
            }
            "/undo" => {
                let message = self.undo_last_operation();
                self.messages.push(format!("assistant> {message}"));
                self.mark_session_dirty();
                return true;
            }
            "/clear" => {
                self.chat_history.clear();
                self.tool_logs.clear();
                self.active_tool_events.clear();
                self.active_stream_preview = None;
                self.messages.clear();
                self.messages.push("assistant> 已清空当前会话显示。输入 /help 查看可用命令。".to_string());
                self.mark_session_dirty();
                return true;
            }
            "/sessions" => {
                self.messages.push(format!("you> {trimmed}"));
                if let Err(error) = self.open_session_picker() {
                    self.messages
                        .push(format!("assistant> 打开会话选择失败：{error}"));
                }
                self.mark_session_dirty();
                return true;
            }
            "/skills" => {
                self.messages.push(format!("you> {trimmed}"));
                let subcommand = trimmed.split_whitespace().nth(1).unwrap_or_default();
                let reply = if subcommand == "reload" {
                    match self.reload_skills() {
                        Ok(count) => {
                            if count == 0 {
                                "skills 已重新加载，但当前仍未发现可用 skill。".to_string()
                            } else {
                                let _ = self.open_skill_picker();
                                format!("skills 已重新加载，共 {} 个。", count)
                            }
                        }
                        Err(error) => format!("skills 重新加载失败：{error}"),
                    }
                } else if subcommand == "list" {
                    self.skills_summary()
                } else if self.skills.is_empty() {
                    self.skills_summary()
                } else {
                    match self.open_skill_picker() {
                        Ok(()) => "已打开 skills 选择弹窗。Enter 查看详情，r 重新加载。".to_string(),
                        Err(error) => format!("打开 skills 弹窗失败：{error}"),
                    }
                };
                self.messages.push(format!("assistant> {reply}"));
                self.mark_session_dirty();
                return true;
            }
            "/skill" => {
                self.messages.push(format!("you> {trimmed}"));
                let name = trimmed.split_whitespace().nth(1).unwrap_or_default();
                let reply = if name.is_empty() {
                    "用法: /skill <name>".to_string()
                } else {
                    self.skill_details(name)
                };
                self.messages.push(format!("assistant> {reply}"));
                self.mark_session_dirty();
                return true;
            }
            "/session" => {
                self.messages.push(format!("you> {trimmed}"));
                let response = self.handle_session_command(trimmed);
                self.messages.push(format!("assistant> {response}"));
                self.mark_session_dirty();
                return true;
            }
            "/thinking" => {
                self.messages.push(format!("you> {trimmed}"));
                self.show_thinking = parse_toggle_arg(trimmed).unwrap_or(!self.show_thinking);
                self.messages.push(format!(
                    "assistant> thinking block 显示已{}。",
                    if self.show_thinking { "开启" } else { "关闭" }
                ));
                self.mark_session_dirty();
                return true;
            }
            "/tool-details" => {
                self.messages.push(format!("you> {trimmed}"));
                self.show_tool_details = parse_toggle_arg(trimmed).unwrap_or(!self.show_tool_details);
                self.messages.push(format!(
                    "assistant> 工具细节展开已{}。",
                    if self.show_tool_details { "开启" } else { "关闭" }
                ));
                self.mark_session_dirty();
                return true;
            }
            _ => {}
        }

        if command_name == "/tools" {
            let definitions = self.tools.definitions();
            self.messages.push(format!("you> {trimmed}"));
            let subcommand = trimmed.split_whitespace().nth(1).unwrap_or_default();
            let reply = if subcommand == "reload" {
                match self.reload_custom_tools() {
                    Ok(count) => format!(
                        "已重新加载 {} 个自定义工具。当前工具总数：{}。",
                        count,
                        self.tools.definitions().len()
                    ),
                    Err(error) => format!("重新加载自定义工具失败：{error}"),
                }
            } else {
                format!(
                    "当前已注册 {} 个工具（其中 .mybot/tools 自定义工具 {} 个）:\n{}",
                    definitions.len(),
                    self.custom_tools.len(),
                    definitions
                        .into_iter()
                        .map(|definition| format!("- {}: {}", definition.name, definition.description))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
            self.messages.push(format!("assistant> {reply}"));
            self.mark_session_dirty();
            return true;
        }

        if trimmed == "/permissions" {
            self.messages.push("you> /permissions".to_string());
            self.messages.push(format!(
                "assistant> 当前权限配置:\n{}",
                self.permission_summary()
            ));
            self.mark_session_dirty();
            return true;
        }

        let Some(rest) = trimmed.strip_prefix("/tool ") else {
            if trimmed.starts_with('/') {
                self.messages.push(format!("you> {trimmed}"));
                let suggestions = slash_command_suggestions(trimmed)
                    .into_iter()
                    .take(4)
                    .map(|command| command.name)
                    .collect::<Vec<_>>();
                self.messages.push(format!(
                    "assistant> 未知命令：{}{}",
                    command_name,
                    if suggestions.is_empty() {
                        String::new()
                    } else {
                        format!("。你可以试试：{}", suggestions.join("、"))
                    }
                ));
                self.mark_session_dirty();
                return true;
            }
            return false;
        };

        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or_default().trim();
        let raw_input = parts.next().unwrap_or("{}").trim();

        if name.is_empty() {
            self.messages
                .push("assistant> 用法: /tool <name> <json-input>".to_string());
            return true;
        }

        let input = match serde_json::from_str::<Value>(if raw_input.is_empty() { "{}" } else { raw_input }) {
            Ok(input) => input,
            Err(error) => {
                self.messages.push(format!(
                    "you> {submitted}\nassistant> 工具输入不是合法 JSON: {error}"
                ));
                return true;
            }
        };

        self.messages.push(format!("you> {submitted}"));
        let undo_snapshot = self
            .tools
            .capture_undo_snapshot(name, &input, &self.tool_context)
            .ok()
            .flatten();
        match self.tools.execute(name, input, &self.tool_context) {
            Ok(output) => {
                if let Some(snapshot) = undo_snapshot {
                    self.record_workspace_undo(snapshot);
                }
                let pretty = serde_json::to_string_pretty(&output.content)
                    .unwrap_or_else(|_| output.content.to_string());
                self.messages.push(format!(
                    "assistant> 工具 {name} 执行成功：{}\n{}",
                    output.summary, pretty
                ));
                self.mark_session_dirty();
            }
            Err(error) => {
                self.messages
                    .push(format!("assistant> 工具 {name} 执行失败：{error}"));
                self.mark_session_dirty();
            }
        }

        true
    }

    fn autocomplete_command(&mut self) {
        let Some(completed) = autocomplete_slash_command_selected(
            self.editor.text(),
            self.command_suggestion_index,
        ) else {
            return;
        };

        self.editor.set_text(completed);
        self.reset_command_suggestion_selection();
    }

    fn try_select_previous_command_suggestion(&mut self) -> bool {
        let suggestions = self.command_suggestions();
        if suggestions.is_empty() {
            return false;
        }

        if self.command_suggestion_index == 0 {
            self.command_suggestion_index = suggestions.len().saturating_sub(1);
        } else {
            self.command_suggestion_index -= 1;
        }
        true
    }

    fn try_select_next_command_suggestion(&mut self) -> bool {
        let suggestions = self.command_suggestions();
        if suggestions.is_empty() {
            return false;
        }

        self.command_suggestion_index = (self.command_suggestion_index + 1) % suggestions.len();
        true
    }

    fn reset_command_suggestion_selection(&mut self) {
        self.command_suggestion_index = 0;
    }

    fn handle_ctrl_c(&mut self) -> bool {
        if self.ctrl_c_armed {
            return true;
        }

        self.ctrl_c_armed = true;

        if !self.editor.is_empty() {
            self.editor.clear();
        }

        false
    }

    fn handle_tool_approval_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if self.keybindings.matches(&key, Action::ApproveTool) {
            self.respond_to_tool_approval(ToolApprovalDecision::Approve);
            return false;
        }

        if self.keybindings.matches(&key, Action::AlwaysAllowTool) {
            self.always_allow_current_tool();
            self.respond_to_tool_approval(ToolApprovalDecision::Approve);
            return false;
        }

        if self.keybindings.matches(&key, Action::AlwaysDenyTool) {
            self.always_deny_current_tool();
            self.respond_to_tool_approval(ToolApprovalDecision::Reject);
            return false;
        }

        if self.keybindings.matches(&key, Action::RejectTool) {
            self.respond_to_tool_approval(ToolApprovalDecision::Reject);
            return false;
        }

        false
    }

    fn handle_session_picker_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if self.keybindings.matches(&key, Action::CloseConfig)
            || self.keybindings.matches(&key, Action::ClearInput)
        {
            self.session_picker = None;
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateUp) {
            if let Some(picker) = self.session_picker.as_mut() {
                if picker.selected == 0 {
                    picker.selected = picker.sessions.len().saturating_sub(1);
                } else {
                    picker.selected -= 1;
                }
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateDown) {
            if let Some(picker) = self.session_picker.as_mut()
                && !picker.sessions.is_empty()
            {
                picker.selected = (picker.selected + 1) % picker.sessions.len();
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::SubmitInput) {
            if let Some(selected_id) = self
                .session_picker
                .as_ref()
                .and_then(|picker| picker.sessions.get(picker.selected))
                .map(|session| session.id.clone())
            {
                let label = match self.switch_session(&selected_id) {
                    Ok(label) => label,
                    Err(error) => format!("切换会话失败：{error}"),
                };
                self.session_picker = None;
                self.messages.push(format!("assistant> {label}"));
                self.mark_session_dirty();
            }
            return false;
        }

        if let KeyCode::Char('r') = key.code {
            if let Some((session_id, title)) = self
                .session_picker
                .as_ref()
                .and_then(|picker| picker.sessions.get(picker.selected))
                .map(|session| (session.id.clone(), session.title.clone()))
            {
                let mut editor = InputEditor::new();
                editor.set_text(title.clone());
                self.session_rename = Some(SessionRenameState {
                    session_id,
                    original_title: title,
                    editor,
                });
            }
            return false;
        }

        false
    }

    fn handle_skill_picker_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if self.keybindings.matches(&key, Action::CloseConfig)
            || self.keybindings.matches(&key, Action::ClearInput)
        {
            self.skill_picker = None;
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateUp) {
            if let Some(picker) = self.skill_picker.as_mut() {
                if picker.selected == 0 {
                    picker.selected = picker.skills.len().saturating_sub(1);
                } else {
                    picker.selected -= 1;
                }
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateDown) {
            if let Some(picker) = self.skill_picker.as_mut()
                && !picker.skills.is_empty()
            {
                picker.selected = (picker.selected + 1) % picker.skills.len();
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::SubmitInput) {
            if let Some(name) = self
                .skill_picker
                .as_ref()
                .and_then(|picker| picker.skills.get(picker.selected))
                .map(|skill| skill.name.clone())
            {
                let reply = self.skill_details(&name);
                self.skill_picker = None;
                self.messages.push(format!("you> /skill {name}"));
                self.messages.push(format!("assistant> {reply}"));
                self.mark_session_dirty();
                self.scroll_conversation_to_bottom();
            }
            return false;
        }

        if let KeyCode::Char('r') = key.code {
            let selected_name = self
                .skill_picker
                .as_ref()
                .and_then(|picker| picker.skills.get(picker.selected))
                .map(|skill| skill.name.clone());

            let message = match self.reload_skills() {
                Ok(count) => {
                    let _ = self.open_skill_picker();
                    if let Some(name) = selected_name {
                        if let Some(picker) = self.skill_picker.as_mut() {
                            picker.selected = picker
                                .skills
                                .iter()
                                .position(|skill| skill.name == name)
                                .unwrap_or(0);
                        }
                    }
                    format!("skills 已重新加载，共 {} 个。", count)
                }
                Err(error) => format!("skills 重新加载失败：{error}"),
            };

            self.messages.push(format!("assistant> {message}"));
            self.mark_session_dirty();
            self.scroll_conversation_to_bottom();
            return false;
        }

        false
    }

    fn handle_session_rename_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let Some(rename) = self.session_rename.as_mut() else {
            return false;
        };

        if self.keybindings.matches(&key, Action::CloseConfig)
            || self.keybindings.matches(&key, Action::ClearInput)
        {
            self.session_rename = None;
            return false;
        }

        if self.keybindings.matches(&key, Action::SubmitInput) {
            let new_title = rename.editor.text().trim().to_string();
            if new_title.is_empty() {
                self.messages
                    .push("assistant> 会话名称不能为空。".to_string());
                self.mark_session_dirty();
                self.session_rename = None;
                return false;
            }

            let session_id = rename.session_id.clone();
            self.session_rename = None;
            let response = match self.rename_session(&session_id, &new_title) {
                Ok(label) => format!("会话已重命名为：{label}"),
                Err(error) => format!("重命名会话失败：{error}"),
            };
            self.messages.push(format!("assistant> {response}"));
            self.mark_session_dirty();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLeft) {
            rename.editor.move_left();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveRight) {
            rename.editor.move_right();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLineStart) {
            rename.editor.move_to_line_start();
            return false;
        }

        if self.keybindings.matches(&key, Action::MoveLineEnd) {
            rename.editor.move_to_line_end();
            return false;
        }

        if self.keybindings.matches(&key, Action::DeleteBackward) {
            rename.editor.delete_before_cursor();
            return false;
        }

        if self.keybindings.matches(&key, Action::DeleteForward) {
            rename.editor.delete_at_cursor();
            return false;
        }

        if let KeyCode::Char(c) = key.code
            && !key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            rename.editor.insert_char(c);
            return false;
        }

        false
    }

    fn respond_to_tool_approval(&mut self, decision: ToolApprovalDecision) {
        let Some(request) = self.pending_tool_approval.take() else {
            return;
        };

        if let Some(sender) = &self.pending_approval_tx {
            let _ = sender.send(decision);
        }

        let outcome = match decision {
            ToolApprovalDecision::Approve => "已批准",
            ToolApprovalDecision::Reject => "已拒绝",
        };
        self.messages.push(format!(
            "assistant> 工具审批：{} {}",
            outcome, request.tool
        ));
        self.mark_session_dirty();
        self.scroll_conversation_to_bottom();
    }

    fn always_allow_current_tool(&mut self) {
        let Some(request) = self.pending_tool_approval.as_ref() else {
            return;
        };

        if let Ok(mut allowed) = self.session_allowed_tools.lock() {
            allowed.insert(request.tool.clone());
        }
        if let Ok(mut denied) = self.session_denied_tools.lock() {
            denied.remove(&request.tool);
        }

        self.messages.push(format!(
            "assistant> 本次会话内已记住：始终允许工具 {}",
            request.tool
        ));
        self.mark_session_dirty();
        self.scroll_conversation_to_bottom();
    }

    fn always_deny_current_tool(&mut self) {
        let Some(request) = self.pending_tool_approval.as_ref() else {
            return;
        };

        if let Ok(mut denied) = self.session_denied_tools.lock() {
            denied.insert(request.tool.clone());
        }
        if let Ok(mut allowed) = self.session_allowed_tools.lock() {
            allowed.remove(&request.tool);
        }

        self.messages.push(format!(
            "assistant> 本次会话内已记住：始终拒绝工具 {}",
            request.tool
        ));
        self.mark_session_dirty();
        self.scroll_conversation_to_bottom();
    }

    fn permission_summary(&self) -> String {
        let mut lines = self.tool_permissions.describe_lines();

        let mut session_allowed = self
            .session_allowed_tools
            .lock()
            .map(|tools| tools.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        session_allowed.sort();

        let mut session_denied = self
            .session_denied_tools
            .lock()
            .map(|tools| tools.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        session_denied.sort();

        lines.push(String::new());
        lines.push(format!(
            "session allow = {}",
            if session_allowed.is_empty() {
                "(empty)".to_string()
            } else {
                session_allowed.join(", ")
            }
        ));
        lines.push(format!(
            "session deny = {}",
            if session_denied.is_empty() {
                "(empty)".to_string()
            } else {
                session_denied.join(", ")
            }
        ));

        lines.join("\n")
    }

    fn push_tool_log(&mut self, title: String, events: Vec<String>) {
        let index = self.tool_logs.len();
        self.tool_logs.push(ToolLogSection { title, events });
        self.messages
            .push(format!("{TOOL_LOG_MARKER_PREFIX}{index}"));
        self.mark_session_dirty();
    }

    fn open_session_picker(&mut self) -> AppResult<()> {
        let sessions = self.session_store.list_sessions()?;
        if sessions.is_empty() {
            self.messages
                .push("assistant> 当前还没有会话。使用 /session new 创建一个。".to_string());
            self.mark_session_dirty();
            return Ok(());
        }

        let selected = sessions
            .iter()
            .position(|session| session.id == self.session_id)
            .unwrap_or(0);
        self.session_picker = Some(SessionPickerState { sessions, selected });
        Ok(())
    }

    fn open_skill_picker(&mut self) -> AppResult<()> {
        let skills = self
            .skills
            .list()
            .into_iter()
            .map(|skill| SkillPickerEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                path: skill.path.to_string_lossy().into_owned(),
            })
            .collect::<Vec<_>>();

        if skills.is_empty() {
            return Err("当前未发现可用 skill".into());
        }

        self.skill_picker = Some(SkillPickerState { skills, selected: 0 });
        Ok(())
    }

    fn push_undo_snapshot(&mut self) {
        if self.pending_response.is_some() || self.pending_tool_approval.is_some() {
            return;
        }

        self.undo_stack.push(UndoSnapshot {
            messages: self.messages.clone(),
            chat_history: self.chat_history.clone(),
            tool_logs: self.tool_logs.clone(),
            workspace_undo: Vec::new(),
            session_title: self.session_title.clone(),
            show_thinking: self.show_thinking,
            show_tool_details: self.show_tool_details,
        });

        if self.undo_stack.len() > 32 {
            let excess = self.undo_stack.len() - 32;
            self.undo_stack.drain(0..excess);
        }
    }

    fn undo_last_operation(&mut self) -> String {
        if self.pending_response.is_some() || self.pending_tool_approval.is_some() {
            return "当前有请求或审批进行中，暂时不能撤销。".to_string();
        }

        let Some(snapshot) = self.undo_stack.pop() else {
            return "没有可撤销的操作。".to_string();
        };

        for workspace_undo in snapshot.workspace_undo.iter().rev() {
            if let Err(error) = apply_workspace_undo_snapshot(workspace_undo, &self.tool_context) {
                return format!(
                    "文件回滚失败（{} {}）：{error}",
                    workspace_undo.tool, workspace_undo.summary
                );
            }
        }

        self.messages = snapshot.messages;
        self.chat_history = snapshot.chat_history;
        self.tool_logs = snapshot.tool_logs;
        self.session_title = snapshot.session_title;
        self.show_thinking = snapshot.show_thinking;
        self.show_tool_details = snapshot.show_tool_details;
        self.active_tool_events.clear();
        self.active_stream_preview = None;
        self.session_picker = None;
        self.mark_session_dirty();
        self.scroll_conversation_to_bottom();
        "已撤销上一次操作。".to_string()
    }

    fn record_workspace_undo(&mut self, snapshot: WorkspaceUndoSnapshot) {
        if let Some(last) = self.undo_stack.last_mut() {
            last.workspace_undo.push(snapshot);
        }
    }

    fn handle_session_command(&mut self, command: &str) -> String {
        let mut parts = command.split_whitespace();
        let _ = parts.next();
        match parts.next() {
            None | Some("current") => format!(
                "当前会话：{}\n/sessions 查看列表；/session new [title] 新建；/session switch <id> 切换。",
                self.session_summary_label()
            ),
            Some("new") => {
                let title = parts.collect::<Vec<_>>().join(" ");
                match self.start_new_session((!title.trim().is_empty()).then_some(title.as_str())) {
                    Ok(label) => format!("已新建并切换到会话：{label}"),
                    Err(error) => format!("新建会话失败：{error}"),
                }
            }
            Some("switch") => {
                let Some(raw_id) = parts.next() else {
                    return "用法: /session switch <id>".to_string();
                };
                match self.switch_session(raw_id) {
                    Ok(label) => format!("已切换到会话：{label}"),
                    Err(error) => format!("切换会话失败：{error}"),
                }
            }
            Some("save") => match self.save_current_session() {
                Ok(()) => format!("会话已保存：{}", self.session_summary_label()),
                Err(error) => format!("保存会话失败：{error}"),
            },
            Some("rename") => {
                let args = parts.collect::<Vec<_>>();
                if args.is_empty() {
                    return "用法: /session rename [id] <title>".to_string();
                }

                let (target_id, title) = if args.len() == 1 {
                    (self.session_id.clone(), args[0].to_string())
                } else {
                    let maybe_id = args[0];
                    match self.session_store.resolve_session_id(maybe_id) {
                        Ok(Some(id)) => (id, args[1..].join(" ")),
                        _ => (self.session_id.clone(), args.join(" ")),
                    }
                };

                if title.trim().is_empty() {
                    return "用法: /session rename [id] <title>".to_string();
                }

                match self.rename_session(&target_id, title.trim()) {
                    Ok(label) => format!("会话已重命名为：{label}"),
                    Err(error) => format!("重命名会话失败：{error}"),
                }
            }
            Some(other) => format!("未知 session 子命令：{other}。可用：current/new/switch/save/rename"),
        }
    }

    fn start_new_session(&mut self, title: Option<&str>) -> AppResult<String> {
        self.save_current_session()?;
        let session = self
            .session_store
            .new_session(title, Self::default_welcome_messages(&self.llm));
        let label = format!("{} ({})", session.title, short_id(&session.id));
        self.restore_session(session);
        self.save_current_session()?;
        Ok(label)
    }

    fn switch_session(&mut self, raw_id: &str) -> AppResult<String> {
        self.save_current_session()?;
        let id = self
            .session_store
            .resolve_session_id(raw_id)?
            .ok_or_else(|| format!("未找到会话: {raw_id}"))?;
        let session = self
            .session_store
            .load_session(&id)?
            .ok_or_else(|| format!("未找到会话: {id}"))?;
        let label = format!("{} ({})", session.title, short_id(&session.id));
        self.restore_session(session);
        self.session_dirty = false;
        Ok(label)
    }

    fn rename_session(&mut self, target_id: &str, title: &str) -> AppResult<String> {
        let resolved_id = self
            .session_store
            .resolve_session_id(target_id)?
            .unwrap_or_else(|| target_id.to_string());
        let session = self
            .session_store
            .rename_session(&resolved_id, title)?
            .ok_or_else(|| format!("未找到会话: {target_id}"))?;

        if resolved_id == self.session_id {
            self.session_title = session.title.clone();
            self.session_dirty = false;
        }

        if let Some(picker) = self.session_picker.as_mut() {
            if let Some(entry) = picker.sessions.iter_mut().find(|entry| entry.id == resolved_id) {
                entry.title = session.title.clone();
                entry.updated_at = session.updated_at;
            }
            picker.sessions.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            picker.selected = picker
                .sessions
                .iter()
                .position(|entry| entry.id == resolved_id)
                .unwrap_or(0);
        }

        Ok(format!("{} ({})", session.title, short_id(&session.id)))
    }

    fn restore_session(&mut self, session: SessionData) {
        self.session_id = session.id;
        self.session_title = session.title;
        self.session_created_at = session.created_at;
        self.messages = session.messages;
        self.chat_history = session.chat_history;
        self.tool_logs = session.tool_logs;
        self.show_thinking = session.show_thinking;
        self.show_tool_details = session.show_tool_details;
        self.active_tool_events.clear();
        self.active_stream_preview = None;
        self.skill_picker = None;
        self.session_picker = None;
        self.session_rename = None;
        self.undo_stack.clear();
        self.pending_response = None;
        self.pending_approval_tx = None;
        self.pending_tool_approval = None;
        self.command_suggestion_index = 0;
        self.conversation_scroll = 0;
        self.scroll_conversation_to_bottom();
    }

    fn save_current_session(&mut self) -> AppResult<()> {
        let session = self.current_session_snapshot();
        self.session_store.save_session(&session)?;
        self.session_dirty = false;
        Ok(())
    }

    fn persist_session_if_dirty(&mut self) {
        if !self.session_dirty || self.pending_response.is_some() {
            return;
        }

        if self.save_current_session().is_err() {
            self.session_dirty = true;
        }
    }

    fn current_session_snapshot(&self) -> SessionData {
        SessionData {
            id: self.session_id.clone(),
            title: self.session_title.clone(),
            created_at: self.session_created_at,
            updated_at: now_unix_seconds(),
            messages: self.messages.clone(),
            chat_history: self.chat_history.clone(),
            tool_logs: self.tool_logs.clone(),
            show_thinking: self.show_thinking,
            show_tool_details: self.show_tool_details,
        }
    }

    fn update_session_title_from_input(&mut self, input: &str) {
        if !self.chat_history.is_empty() {
            return;
        }

        if !self.session_title.starts_with("Session ") {
            return;
        }

        let mut text = input.trim().replace('\n', " ");
        if text.chars().count() > 32 {
            text = text.chars().take(32).collect::<String>() + "...";
        }
        if !text.is_empty() {
            self.session_title = text;
        }
    }

    fn mark_session_dirty(&mut self) {
        self.session_dirty = true;
    }

    fn short_session_id(&self) -> String {
        short_id(&self.session_id)
    }

    fn skills_summary(&self) -> String {
        if self.skills.is_empty() {
            return "当前未发现 skills。可放在 .mybot/skills，或兼容 OpenCode 的 .opencode/skills、.claude/skills、.agents/skills 以及对应的全局目录。".to_string();
        }

        format!(
            "当前共发现 {} 个 skills:\n{}",
            self.skills.len(),
            self.skills
                .list()
                .into_iter()
                .map(|skill| format!("- {}: {}", skill.name, skill.description))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn reload_skills(&mut self) -> AppResult<usize> {
        let store = SkillStore::discover(self.tool_context.workspace_root(), &self.runtime_root)?;
        let count = store.len();
        let skills = Arc::new(store);
        self.tools = ToolRegistry::with_extensions(skills.clone(), self.custom_tools.clone());
        self.skills = skills;
        Ok(count)
    }

    fn reload_custom_tools(&mut self) -> AppResult<usize> {
        let store = CustomToolStore::discover(&self.runtime_root)?;
        let count = store.len();
        let custom_tools = Arc::new(store);
        self.tools = ToolRegistry::with_extensions(self.skills.clone(), custom_tools.clone());
        self.custom_tools = custom_tools;
        Ok(count)
    }

    fn skill_details(&self, name: &str) -> String {
        let Some(skill) = self.skills.get(name) else {
            return format!("未找到 skill：{}", name);
        };

        let mut lines = vec![
            format!("name: {}", skill.name),
            format!("description: {}", skill.description),
            format!("path: {}", skill.path.display()),
        ];
        if let Some(license) = &skill.license {
            lines.push(format!("license: {}", license));
        }
        if let Some(compatibility) = &skill.compatibility {
            lines.push(format!("compatibility: {}", compatibility));
        }
        if !skill.metadata.is_empty() {
            lines.push(format!(
                "metadata: {}",
                skill.metadata
                    .iter()
                    .map(|(key, value)| format!("{}={}", key, value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(String::new());
        lines.push(skill.content.clone());
        lines.join("\n")
    }

    fn handle_config_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let Some(mut editor) = self.config_editor.take() else {
            return false;
        };

        match editor.handle_key(&key, &self.keybindings) {
            Some(ConfigEvent::Close) => {
                self.messages.push(if editor.dirty() {
                    "assistant> 已关闭配置界面，未保存的修改仍保留在文件外。".to_string()
                } else {
                    "assistant> 已关闭配置界面。".to_string()
                });
                self.scroll_conversation_to_bottom();
                false
            }
            Some(ConfigEvent::Saved(store)) => {
                let active_config = match store.active_config() {
                    Ok(config) => config,
                    Err(error) => {
                        self.messages.push(format!(
                            "assistant> 配置文件已保存，但读取当前激活配置失败：{error}"
                        ));
                        self.scroll_conversation_to_bottom();
                        return false;
                    }
                };

                match LlmClient::new(active_config) {
                    Ok(llm) => {
                        self.llm = llm;
                        self.llm_profiles = store;
                        self.messages.push(format!(
                            "assistant> 配置已保存并生效，当前 profile：{}，provider：{}，模型：{}。",
                            self.profile_name(),
                            self.llm.provider_name(),
                            self.llm.model_name()
                        ));
                    }
                    Err(error) => {
                        self.messages.push(format!(
                            "assistant> 配置文件已保存，但重载客户端失败：{error}"
                        ));
                    }
                }
                self.scroll_conversation_to_bottom();
                false
            }
            None => {
                self.config_editor = Some(editor);
                false
            }
        }
    }

    fn open_config_editor(&mut self) {
        if self.pending_response.is_some() {
            self.messages
                .push("assistant> 当前有请求在处理中，暂时不能打开配置界面。".to_string());
            self.scroll_conversation_to_bottom();
            return;
        }

        self.config_editor = Some(ConfigEditor::new(
            &self.llm_profiles,
            self.llm_config_path.clone(),
        ));
    }

    fn page_scroll_amount(&self) -> u16 {
        self.conversation_viewport.height.saturating_sub(1).max(1)
    }

    fn scroll_conversation_up(&mut self, amount: u16) {
        self.conversation_scroll = self.conversation_scroll.saturating_sub(amount);
    }

    fn scroll_conversation_down(&mut self, amount: u16) {
        self.conversation_scroll = (self.conversation_scroll + amount).min(self.max_conversation_scroll());
    }

    fn scroll_conversation_to_bottom(&mut self) {
        self.conversation_scroll = self.max_conversation_scroll();
    }

    fn clamp_conversation_scroll(&mut self) {
        self.conversation_scroll = self.conversation_scroll.min(self.max_conversation_scroll());
    }

    fn total_conversation_lines(&self, width: usize) -> usize {
        crate::ui::conversation_plain_lines(self)
            .into_iter()
            .map(|line| wrapped_line_count(&line, width))
            .sum::<usize>()
            .max(1)
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }

    text.split('\n')
        .map(|line| {
            let length = line.chars().count();
            length.max(1).div_ceil(width)
        })
        .sum::<usize>()
        .max(1)
}

fn parse_toggle_arg(command: &str) -> Option<bool> {
    match command.split_whitespace().nth(1) {
        Some("on") | Some("enable") | Some("true") => Some(true),
        Some("off") | Some("disable") | Some("false") => Some(false),
        Some("toggle") | None => None,
        Some(_) => None,
    }
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

