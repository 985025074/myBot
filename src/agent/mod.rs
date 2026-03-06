use serde::Deserialize;
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender};

use crate::{
    app::AppResult,
    config::PermissionMode,
    llm::{ChatMessage, LlmClient},
    tools::{ToolContext, ToolRegistry},
};

#[derive(Debug, Clone)]
pub struct AgentExecutor {
    llm: LlmClient,
    tools: ToolRegistry,
    tool_context: ToolContext,
    max_steps: usize,
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub final_reply: String,
    pub events: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolApprovalRequest {
    pub step: usize,
    pub tool: String,
    pub input: Value,
    pub thought: Option<String>,
}

#[derive(Debug)]
pub enum AgentThreadMessage {
    ApprovalRequired(ToolApprovalRequest),
    Finished(AppResult<AgentRunResult>),
}

#[derive(Debug, Clone, Copy)]
pub enum ToolApprovalDecision {
    Approve,
    Reject,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentResponse {
    Final {
        message: String,
    },
    ToolCall {
        tool: String,
        input: Value,
        #[serde(default)]
        thought: Option<String>,
    },
}

impl AgentExecutor {
    pub fn new(llm: LlmClient, tools: ToolRegistry, tool_context: ToolContext) -> Self {
        Self {
            llm,
            tools,
            tool_context,
            max_steps: 8,
        }
    }

    pub fn run(
        &self,
        history: &[ChatMessage],
        permissions: impl Fn(&str) -> PermissionMode,
        event_tx: Sender<AgentThreadMessage>,
        decision_rx: Receiver<ToolApprovalDecision>,
    ) -> AppResult<AgentRunResult> {
        let definitions = self.tools.definitions();
        let tools_json = serde_json::to_string_pretty(&definitions)?;
        let mut scratchpad = Vec::new();
        let mut events = Vec::new();

        for step in 1..=self.max_steps {
            let mut prompt_history = history.to_vec();
            prompt_history.push(ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "You can use tools to solve the user's request. Available tools:\n{}\n\nReply with JSON only.\nIf you need a tool, reply exactly in this shape:\n{{\"type\":\"tool_call\",\"tool\":\"read_file\",\"input\":{{...}},\"thought\":\"optional short reason\"}}\nIf you are ready to answer the user, reply exactly in this shape:\n{{\"type\":\"final\",\"message\":\"your final answer\"}}\nRules:\n- Use only the listed tool names.\n- Keep tool inputs valid JSON objects.\n- Prefer tools when you need workspace facts.\n- When enough information is gathered, return type=final."
                    ,
                    tools_json
                ),
            });
            prompt_history.extend(scratchpad.clone());

            let raw = self.llm.send_chat(&prompt_history)?;
            match parse_agent_response(&raw) {
                Some(AgentResponse::Final { message }) => {
                    return Ok(AgentRunResult {
                        final_reply: message,
                        events,
                    });
                }
                Some(AgentResponse::ToolCall {
                    tool,
                    input,
                    thought,
                }) => {
                    let thought_text = thought
                        .as_ref()
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| format!(" · {value}"))
                        .unwrap_or_default();
                    events.push(format!("tool> step {step} 调用 {tool}{thought_text}"));

                    let mode = permissions(&tool);
                    let tool_result = match mode {
                        PermissionMode::Allow => {
                            execute_tool(&self.tools, &self.tool_context, &mut events, &tool, input.clone())
                        }
                        PermissionMode::Deny => {
                            events.push(format!("tool> {tool} 已被权限系统拒绝"));
                            serde_json::json!({
                                "ok": false,
                                "tool": tool,
                                "input": input,
                                "error": "tool execution denied by permission policy",
                            })
                        }
                        PermissionMode::Ask => {
                            event_tx
                                .send(AgentThreadMessage::ApprovalRequired(ToolApprovalRequest {
                                    step,
                                    tool: tool.clone(),
                                    input: input.clone(),
                                    thought: thought.clone(),
                                }))
                                .map_err(|_| "failed to send tool approval request")?;

                            match decision_rx.recv().map_err(|_| "approval channel disconnected")? {
                                ToolApprovalDecision::Approve => execute_tool(
                                    &self.tools,
                                    &self.tool_context,
                                    &mut events,
                                    &tool,
                                    input.clone(),
                                ),
                                ToolApprovalDecision::Reject => {
                                    events.push(format!("tool> {tool} 被用户拒绝执行"));
                                    serde_json::json!({
                                        "ok": false,
                                        "tool": tool,
                                        "input": input,
                                        "error": "tool execution rejected by user",
                                    })
                                }
                            }
                        }
                    };

                    scratchpad.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: raw,
                    });
                    scratchpad.push(ChatMessage {
                        role: "user".to_string(),
                        content: format!(
                            "Tool execution result for step {step}:\n{}\n\nIf more tools are needed, return another tool_call JSON. Otherwise return a final JSON response.",
                            serde_json::to_string_pretty(&tool_result)?
                        ),
                    });
                }
                None => {
                    return Ok(AgentRunResult {
                        final_reply: raw,
                        events,
                    });
                }
            }
        }

        Ok(AgentRunResult {
            final_reply: "已达到工具执行步数上限，请缩小问题范围后重试。".to_string(),
            events,
        })
    }
}

fn execute_tool(
    tools: &ToolRegistry,
    tool_context: &ToolContext,
    events: &mut Vec<String>,
    tool: &str,
    input: Value,
) -> Value {
    match tools.execute(tool, input.clone(), tool_context) {
        Ok(output) => {
            events.push(format!("tool> {}", output.summary));
            serde_json::json!({
                "ok": true,
                "tool": tool,
                "input": input,
                "summary": output.summary,
                "content": output.content,
            })
        }
        Err(error) => {
            events.push(format!("tool> {tool} 执行失败: {error}"));
            serde_json::json!({
                "ok": false,
                "tool": tool,
                "input": input,
                "error": error.to_string(),
            })
        }
    }
}

fn parse_agent_response(raw: &str) -> Option<AgentResponse> {
    let trimmed = raw.trim();
    let candidates = [
        trimmed.to_string(),
        extract_fenced_json(trimmed).unwrap_or_default(),
        extract_json_object(trimmed).unwrap_or_default(),
    ];

    candidates
        .into_iter()
        .filter(|candidate| !candidate.trim().is_empty())
        .find_map(|candidate| serde_json::from_str::<AgentResponse>(&candidate).ok())
}

fn extract_fenced_json(text: &str) -> Option<String> {
    let start = text.find("```")?;
    let rest = &text[start + 3..];
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("```")?;
    Some(rest[..end].trim().to_string())
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end).then(|| text[start..=end].trim().to_string())
}