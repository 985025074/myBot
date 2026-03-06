use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use wait_timeout::ChildExt;

use crate::app::AppResult;

use super::{Tool, ToolContext, ToolDefinition, ToolOutput, ToolRegistry};

const RESERVED_TOOL_NAMES: &[&str] = &[
    "list_files",
    "glob_files",
    "file_stat",
    "make_directory",
    "move_path",
    "delete_path",
    "read_file",
    "write_file",
    "apply_patch",
    "grep_text",
    "run_command",
    "skill",
];

#[derive(Debug, Clone, Default)]
pub struct CustomToolStore {
    tools: Vec<CustomToolManifest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomToolManifest {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default = "default_input_schema")]
    pub input_schema: Value,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(skip)]
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone)]
struct CustomCommandTool {
    manifest: CustomToolManifest,
}

impl CustomToolStore {
    pub fn discover(runtime_root: &Path) -> AppResult<Self> {
        let root = runtime_root.join("tools");
        if !root.exists() {
            return Ok(Self::default());
        }

        let mut tools = Vec::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }

            let content = fs::read_to_string(&path)?;
            let mut manifest = toml::from_str::<CustomToolManifest>(&content)?;
            manifest.manifest_path = path.clone();
            validate_manifest(&manifest)?;
            tools.push(manifest);
        }

        tools.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { tools })
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn list(&self) -> &[CustomToolManifest] {
        &self.tools
    }
}

pub fn register_custom_tools(registry: &mut ToolRegistry, custom_tools: Arc<CustomToolStore>) {
    for manifest in custom_tools.list() {
        registry.register(CustomCommandTool {
            manifest: manifest.clone(),
        });
    }
}

impl Tool for CustomCommandTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.manifest.name.clone(),
            description: format!(
                "{}\n\n[custom tool] manifest={} command={}",
                self.manifest.description,
                self.manifest.manifest_path.display(),
                self.manifest.command
            ),
            input_schema: self.manifest.input_schema.clone(),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let cwd = context.resolve_path(&self.manifest.working_dir)?;
        let timeout = Duration::from_secs(self.manifest.timeout_seconds.max(1));
        let input_text = serde_json::to_string_pretty(&input)?;

        let mut child = Command::new("sh")
            .arg("-lc")
            .arg(&self.manifest.command)
            .current_dir(&cwd)
            .env("MYBOT_TOOL_NAME", &self.manifest.name)
            .env("MYBOT_WORKSPACE_ROOT", context.workspace_root())
            .env("MYBOT_TOOL_INPUT", &input_text)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input_text.as_bytes())?;
        }

        let status = match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "custom tool '{}' timed out after {}s",
                    self.manifest.name, self.manifest.timeout_seconds
                )
                .into());
            }
        };

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Some(mut handle) = child.stdout.take() {
            let _ = handle.read_to_string(&mut stdout);
        }
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }

        if !status.success() {
            return Err(format!(
                "custom tool '{}' failed with exit code {}{}",
                self.manifest.name,
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string()),
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", truncate_output(&stderr))
                }
            )
            .into());
        }

        Ok(parse_custom_tool_output(
            &self.manifest,
            &cwd,
            context,
            &stdout,
            &stderr,
        ))
    }
}

fn parse_custom_tool_output(
    manifest: &CustomToolManifest,
    cwd: &Path,
    context: &ToolContext,
    stdout: &str,
    stderr: &str,
) -> ToolOutput {
    let trimmed = stdout.trim();
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed)
        && let Some(object) = parsed.as_object()
    {
        if let Some(summary) = object.get("summary").and_then(|value| value.as_str()) {
            let mut content = object
                .get("content")
                .cloned()
                .unwrap_or_else(|| parsed.clone());
            if !stderr.trim().is_empty() {
                content = json!({
                    "result": content,
                    "stderr": truncate_output(stderr),
                });
            }
            return ToolOutput {
                summary: summary.to_string(),
                content,
            };
        }

        return ToolOutput {
            summary: format!("custom tool {} completed", manifest.name),
            content: json!({
                "cwd": context.to_relative_display(cwd),
                "stdout": parsed,
                "stderr": truncate_output(stderr),
            }),
        };
    }

    ToolOutput {
        summary: format!("custom tool {} completed", manifest.name),
        content: json!({
            "cwd": context.to_relative_display(cwd),
            "stdout": truncate_output(stdout),
            "stderr": truncate_output(stderr),
        }),
    }
}

fn validate_manifest(manifest: &CustomToolManifest) -> AppResult<()> {
    let regex = Regex::new(r"^[a-z][a-z0-9_\-]*$")?;
    if !regex.is_match(&manifest.name) {
        return Err(format!(
            "invalid custom tool name '{}' in {}",
            manifest.name,
            manifest.manifest_path.display()
        )
        .into());
    }

    if RESERVED_TOOL_NAMES.contains(&manifest.name.as_str()) {
        return Err(format!(
            "custom tool name '{}' is reserved in {}",
            manifest.name,
            manifest.manifest_path.display()
        )
        .into());
    }

    if manifest.description.trim().is_empty() {
        return Err(format!(
            "custom tool '{}' must have a non-empty description",
            manifest.name
        )
        .into());
    }

    if manifest.command.trim().is_empty() {
        return Err(format!(
            "custom tool '{}' must have a non-empty command",
            manifest.name
        )
        .into());
    }

    Ok(())
}

fn default_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "description": "Arbitrary JSON passed to the custom tool via stdin and MYBOT_TOOL_INPUT"
    })
}

fn default_timeout_seconds() -> u64 {
    30
}

fn truncate_output(text: &str) -> String {
    const LIMIT: usize = 8_000;
    if text.len() <= LIMIT {
        text.to_string()
    } else {
        format!("{}\n...[truncated]", &text[..LIMIT])
    }
}
