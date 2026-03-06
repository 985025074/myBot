mod config_editor;
mod input;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::Value;

use crate::{
    agent::{
        AgentExecutor, AgentThreadMessage, ToolApprovalDecision,
        ToolApprovalRequest,
    },
    config::{Action, KeyBindings, LlmConfigStore, ToolPermissionConfig},
    llm::{ChatMessage, LlmClient},
    tools::{ToolContext, ToolRegistry},
};

pub use config_editor::{ConfigEditor, ConfigEvent};
pub use input::InputEditor;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Copy, Default)]
struct Viewport {
    width: u16,
    height: u16,
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
    tool_context: ToolContext,
    chat_history: Vec<ChatMessage>,
    pending_response: Option<Receiver<AgentThreadMessage>>,
    pending_approval_tx: Option<Sender<ToolApprovalDecision>>,
    pending_tool_approval: Option<ToolApprovalRequest>,
    config_editor: Option<ConfigEditor>,
    conversation_viewport: Viewport,
    ctrl_c_armed: bool,
}

impl App {
    pub fn new(
        keybindings: KeyBindings,
        llm: LlmClient,
        llm_profiles: LlmConfigStore,
        llm_config_path: PathBuf,
        tool_permissions: ToolPermissionConfig,
        tools: ToolRegistry,
        tool_context: ToolContext,
    ) -> Self {
        Self {
            editor: InputEditor::new(),
            messages: vec![
                "assistant> 欢迎使用 mybot TUI 骨架。".to_string(),
                format!(
                    "assistant> 已接入 provider 抽象，当前 provider：{}，模型：{}。",
                    llm.provider_name(),
                    llm.model_name()
                ),
                "assistant> 已引入模块化工具系统。输入 /tools 查看工具，或用 /tool <name> <json> 手动调用。"
                    .to_string(),
                "assistant> 现在支持历史输入、多行输入、输入区自动换行和聊天区滚动。"
                    .to_string(),
            ],
            conversation_scroll: 0,
            keybindings,
            llm,
            llm_profiles,
            llm_config_path,
            tool_permissions,
            tools,
            tool_context,
            chat_history: Vec::new(),
            pending_response: None,
            pending_approval_tx: None,
            pending_tool_approval: None,
            config_editor: None,
            conversation_viewport: Viewport::default(),
            ctrl_c_armed: false,
        }
    }

    pub fn tick(&mut self) {
        let Some(receiver) = self.pending_response.take() else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => match message {
                AgentThreadMessage::ApprovalRequired(request) => {
                    self.pending_tool_approval = Some(request);
                    self.pending_response = Some(receiver);
                }
                AgentThreadMessage::Finished(result) => match result {
                    Ok(result) => {
                        self.remove_pending_placeholder();
                        self.pending_approval_tx = None;
                        self.pending_tool_approval = None;
                        for event in result.events {
                            self.messages.push(format!("assistant> {event}"));
                        }
                        self.chat_history.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: result.final_reply.clone(),
                        });
                        self.messages
                            .push(format!("assistant> {}", result.final_reply));
                        self.scroll_conversation_to_bottom();
                    }
                    Err(error) => {
                        self.remove_pending_placeholder();
                        self.pending_approval_tx = None;
                        self.pending_tool_approval = None;
                        self.messages.push(format!("assistant> 请求失败：{error}"));
                        self.scroll_conversation_to_bottom();
                    }
                },
            },
            Err(TryRecvError::Empty) => {
                self.pending_response = Some(receiver);
            }
            Err(TryRecvError::Disconnected) => {
                self.pending_approval_tx = None;
                self.pending_tool_approval = None;
                self.messages
                    .push("assistant> 请求通道已断开，请重试。".to_string());
                self.scroll_conversation_to_bottom();
            }
        }
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

    pub fn key_label(&self, action: Action) -> &str {
        self.keybindings.label(action)
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

        if self.pending_tool_approval.is_some() {
            return self.handle_tool_approval_event(key);
        }

        let modifiers = key.modifiers;

        if self.keybindings.matches(&key, Action::OpenConfig) {
            self.open_config_editor();
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
            if self.editor.is_cursor_on_first_line() {
                self.editor.use_older_history();
            } else {
                self.editor.move_up();
            }
            return false;
        }

        if self.keybindings.matches(&key, Action::NavigateDown) {
            if self.editor.is_cursor_on_last_line() {
                self.editor.use_newer_history();
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
            return false;
        }

        if self.keybindings.matches(&key, Action::DeleteForward) {
            self.editor.delete_at_cursor();
            return false;
        }

        if self.keybindings.matches(&key, Action::InsertNewline) {
            self.editor.insert_newline();
            return false;
        }

        if self.keybindings.matches(&key, Action::SubmitInput) {
            self.submit_input();
            return false;
        }

        if self.keybindings.matches(&key, Action::ClearInput) {
            self.editor.clear();
            return false;
        }

        match key.code {
            KeyCode::Char(c)
                if !modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
            {
                self.editor.insert_char(c);
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

        if self.try_handle_local_command(&submitted) {
            self.scroll_conversation_to_bottom();
            return;
        }

        self.chat_history.push(ChatMessage {
            role: "user".to_string(),
            content: submitted.clone(),
        });
        self.messages.push(format!("you> {submitted}"));
        self.messages.push(format!(
            "assistant> 正在调用 {}，必要时会自动执行工具 ...",
            self.llm.model_name()
        ));
        self.scroll_conversation_to_bottom();

        let agent = AgentExecutor::new(
            self.llm.clone(),
            self.tools.clone(),
            self.tool_context.clone(),
        );
        let history = self.chat_history.clone();
        let permissions = self.tool_permissions.clone();
        let (tx, rx) = mpsc::channel();
        let (decision_tx, decision_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = agent.run(&history, |tool_name| permissions.mode_for(tool_name), tx.clone(), decision_rx);
            let _ = tx.send(AgentThreadMessage::Finished(result));
        });
        self.pending_response = Some(rx);
        self.pending_approval_tx = Some(decision_tx);
    }

    fn try_handle_local_command(&mut self, submitted: &str) -> bool {
        let trimmed = submitted.trim();

        if trimmed == "/tools" {
            let definitions = self.tools.definitions();
            self.messages.push("you> /tools".to_string());
            self.messages.push(format!(
                "assistant> 当前已注册 {} 个工具:\n{}",
                definitions.len(),
                definitions
                    .into_iter()
                    .map(|definition| format!("- {}: {}", definition.name, definition.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            return true;
        }

        let Some(rest) = trimmed.strip_prefix("/tool ") else {
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
        match self.tools.execute(name, input, &self.tool_context) {
            Ok(output) => {
                let pretty = serde_json::to_string_pretty(&output.content)
                    .unwrap_or_else(|_| output.content.to_string());
                self.messages.push(format!(
                    "assistant> 工具 {name} 执行成功：{}\n{}",
                    output.summary, pretty
                ));
            }
            Err(error) => {
                self.messages
                    .push(format!("assistant> 工具 {name} 执行失败：{error}"));
            }
        }

        true
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

        if self.keybindings.matches(&key, Action::RejectTool) {
            self.respond_to_tool_approval(ToolApprovalDecision::Reject);
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
        self.scroll_conversation_to_bottom();
    }

    fn remove_pending_placeholder(&mut self) {
        if let Some(last) = self.messages.last()
            && last.starts_with("assistant> 正在调用 ")
        {
            self.messages.pop();
        }
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
        self.messages
            .iter()
            .map(|message| wrapped_line_count(message, width) + 1)
            .sum()
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
