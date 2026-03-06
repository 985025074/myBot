use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use glob::Pattern;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use wait_timeout::ChildExt;
use walkdir::WalkDir;

use crate::{
    app::AppResult,
    skills::SkillStore,
    tools::{Tool, ToolContext, ToolDefinition, ToolOutput, ToolRegistry},
};

pub fn register_builtin_tools(registry: &mut ToolRegistry, skills: Arc<SkillStore>) {
    registry.register(ListFilesTool);
    registry.register(GlobFilesTool);
    registry.register(FileStatTool);
    registry.register(MakeDirectoryTool);
    registry.register(MovePathTool);
    registry.register(DeletePathTool);
    registry.register(ReadFileTool);
    registry.register(WriteFileTool);
    registry.register(ApplyPatchTool);
    registry.register(GrepTextTool);
    registry.register(RunCommandTool);
    registry.register(SkillTool { skills });
}

struct ListFilesTool;
struct GlobFilesTool;
struct FileStatTool;
struct MakeDirectoryTool;
struct MovePathTool;
struct DeletePathTool;
struct ReadFileTool;
struct WriteFileTool;
struct ApplyPatchTool;
struct GrepTextTool;
struct RunCommandTool;
struct SkillTool {
    skills: Arc<SkillStore>,
}

#[derive(Debug, Deserialize)]
struct ListFilesInput {
    #[serde(default)]
    path: String,
    #[serde(default = "default_max_entries")]
    max_entries: usize,
}

#[derive(Debug, Deserialize)]
struct GlobFilesInput {
    pattern: String,
    #[serde(default = "default_max_entries")]
    max_entries: usize,
}

#[derive(Debug, Deserialize)]
struct ReadFileInput {
    path: String,
    #[serde(default = "default_start_line")]
    start_line: usize,
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PathInput {
    path: String,
}

#[derive(Debug, Deserialize)]
struct MakeDirectoryInput {
    path: String,
    #[serde(default = "default_recursive_true")]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct MovePathInput {
    source: String,
    destination: String,
    #[serde(default)]
    create_parent_dirs: bool,
}

#[derive(Debug, Deserialize)]
struct DeletePathInput {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct WriteFileInput {
    path: String,
    content: String,
    #[serde(default)]
    append: bool,
    #[serde(default)]
    create_parent_dirs: bool,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchInput {
    path: String,
    edits: Vec<PatchEdit>,
}

#[derive(Debug, Deserialize)]
struct PatchEdit {
    find: String,
    replace: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Debug, Deserialize)]
struct GrepTextInput {
    query: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize)]
struct RunCommandInput {
    command: String,
    #[serde(default)]
    path: String,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct SkillInput {
    name: String,
}

impl Tool for ListFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files and directories under the workspace or a subdirectory.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Relative path inside the workspace"},
                    "max_entries": {"type": "integer", "minimum": 1, "description": "Maximum number of entries to return"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: ListFilesInput = serde_json::from_value(input)?;
        let root = context.resolve_path(&input.path)?;
        let mut entries = Vec::new();

        for entry in WalkDir::new(&root)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| !should_skip(entry.path()))
            .filter_map(Result::ok)
            .take(input.max_entries)
        {
            let entry_type = if entry.file_type().is_dir() { "dir" } else { "file" };
            entries.push(json!({
                "path": context.to_relative_display(entry.path()),
                "type": entry_type,
            }));
        }

        Ok(ToolOutput {
            summary: format!("listed {} entries", entries.len()),
            content: json!({
                "root": context.to_relative_display(&root),
                "entries": entries,
            }),
        })
    }
}

impl Tool for GlobFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "glob_files".to_string(),
            description: "Find workspace files using a glob pattern like src/**/*.rs or **/*.toml.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": {"type": "string"},
                    "max_entries": {"type": "integer", "minimum": 1}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: GlobFilesInput = serde_json::from_value(input)?;
        let pattern = Pattern::new(&input.pattern)?;
        let mut matches = Vec::new();

        for entry in WalkDir::new(context.resolve_path("")?)
            .into_iter()
            .filter_entry(|entry| !should_skip(entry.path()))
            .filter_map(Result::ok)
        {
            let relative = context.to_relative_display(entry.path());
            if relative == "." {
                continue;
            }

            if pattern.matches(&relative) {
                matches.push(json!({
                    "path": relative,
                    "type": if entry.file_type().is_dir() { "dir" } else { "file" },
                }));
            }

            if matches.len() >= input.max_entries {
                break;
            }
        }

        Ok(ToolOutput {
            summary: format!("matched {} paths", matches.len()),
            content: json!({
                "pattern": input.pattern,
                "matches": matches,
            }),
        })
    }
}

impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read part or all of a UTF-8 text file from the workspace.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string", "description": "Relative file path inside the workspace"},
                    "start_line": {"type": "integer", "minimum": 1},
                    "end_line": {"type": "integer", "minimum": 1}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: ReadFileInput = serde_json::from_value(input)?;
        let path = context.resolve_path(&input.path)?;
        let content = fs::read_to_string(&path)?;
        let start_line = input.start_line.max(1);
        let end_line = input.end_line.unwrap_or(usize::MAX).max(start_line);

        let lines = content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line_number = index + 1;
                (line_number >= start_line && line_number <= end_line)
                    .then(|| json!({"line": line_number, "text": line}))
            })
            .collect::<Vec<_>>();

        Ok(ToolOutput {
            summary: format!(
                "read {} lines from {}",
                lines.len(),
                context.to_relative_display(&path)
            ),
            content: json!({
                "path": context.to_relative_display(&path),
                "lines": lines,
            }),
        })
    }
}

impl Tool for FileStatTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_stat".to_string(),
            description: "Inspect metadata for a workspace file or directory.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: PathInput = serde_json::from_value(input)?;
        let path = context.resolve_path(&input.path)?;
        let metadata = fs::metadata(&path)?;
        let file_type = if metadata.is_dir() {
            "dir"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };

        Ok(ToolOutput {
            summary: format!("inspected {}", context.to_relative_display(&path)),
            content: json!({
                "path": context.to_relative_display(&path),
                "type": file_type,
                "size": metadata.len(),
                "readonly": metadata.permissions().readonly(),
                "modified_unix_seconds": metadata
                    .modified()
                    .ok()
                    .and_then(system_time_to_unix_seconds),
                "created_unix_seconds": metadata
                    .created()
                    .ok()
                    .and_then(system_time_to_unix_seconds),
            }),
        })
    }
}

impl Tool for MakeDirectoryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "make_directory".to_string(),
            description: "Create a directory inside the workspace.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "recursive": {"type": "boolean", "description": "Create parent directories recursively"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: MakeDirectoryInput = serde_json::from_value(input)?;
        let path = context.prepare_path(&input.path)?;

        if input.recursive {
            fs::create_dir_all(&path)?;
        } else {
            fs::create_dir(&path)?;
        }

        Ok(ToolOutput {
            summary: format!("created directory {}", context.to_relative_display(&path)),
            content: json!({
                "path": context.to_relative_display(&path),
                "recursive": input.recursive,
            }),
        })
    }
}

impl Tool for MovePathTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "move_path".to_string(),
            description: "Move or rename a file or directory within the workspace.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["source", "destination"],
                "properties": {
                    "source": {"type": "string"},
                    "destination": {"type": "string"},
                    "create_parent_dirs": {"type": "boolean"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: MovePathInput = serde_json::from_value(input)?;
        let source = context.resolve_path(&input.source)?;
        let destination = context.prepare_path(&input.destination)?;

        if input.create_parent_dirs {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
        }

        fs::rename(&source, &destination)?;

        Ok(ToolOutput {
            summary: format!(
                "moved {} to {}",
                context.to_relative_display(&source),
                context.to_relative_display(&destination)
            ),
            content: json!({
                "source": context.to_relative_display(&source),
                "destination": context.to_relative_display(&destination),
            }),
        })
    }
}

impl Tool for DeletePathTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_path".to_string(),
            description: "Delete a file or directory inside the workspace.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": {"type": "string"},
                    "recursive": {"type": "boolean", "description": "Required for non-empty directories"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: DeletePathInput = serde_json::from_value(input)?;
        let path = context.resolve_path(&input.path)?;
        let metadata = fs::metadata(&path)?;

        if metadata.is_dir() {
            if input.recursive {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_dir(&path)?;
            }
        } else {
            fs::remove_file(&path)?;
        }

        Ok(ToolOutput {
            summary: format!("deleted {}", context.to_relative_display(&path)),
            content: json!({
                "path": context.to_relative_display(&path),
                "recursive": input.recursive,
            }),
        })
    }
}

impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write or append UTF-8 text content to a workspace file.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                    "append": {"type": "boolean"},
                    "create_parent_dirs": {"type": "boolean"}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: WriteFileInput = serde_json::from_value(input)?;
        let path = context.prepare_path(&input.path)?;

        if input.create_parent_dirs {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
        }

        if input.append {
            if !path.exists() && !input.create_parent_dirs {
                if let Some(parent) = path.parent() {
                    if !parent.exists() {
                        return Err("parent directory does not exist; set create_parent_dirs=true".into());
                    }
                }
            }
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(input.content.as_bytes())?;
        } else {
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    if !parent.exists() {
                        return Err("parent directory does not exist; set create_parent_dirs=true".into());
                    }
                }
            }
            fs::write(&path, input.content.as_bytes())?;
        }

        Ok(ToolOutput {
            summary: format!(
                "{} {}",
                if input.append { "appended to" } else { "wrote" },
                context.to_relative_display(&path)
            ),
            content: json!({
                "path": context.to_relative_display(&path),
                "bytes": input.content.len(),
                "append": input.append,
            }),
        })
    }
}

impl Tool for ApplyPatchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply exact string replacement edits to a workspace text file.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "edits"],
                "properties": {
                    "path": {"type": "string"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["find", "replace"],
                            "properties": {
                                "find": {"type": "string"},
                                "replace": {"type": "string"},
                                "replace_all": {"type": "boolean"}
                            }
                        }
                    }
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: ApplyPatchInput = serde_json::from_value(input)?;
        if input.edits.is_empty() {
            return Err("edits cannot be empty".into());
        }

        let path = context.resolve_path(&input.path)?;
        let mut content = fs::read_to_string(&path)?;
        let mut applied = Vec::new();

        for (index, edit) in input.edits.iter().enumerate() {
            if edit.find.is_empty() {
                return Err(format!("edit {} has empty find string", index + 1).into());
            }

            let count = content.matches(&edit.find).count();
            if count == 0 {
                return Err(format!(
                    "edit {} could not find target text in {}",
                    index + 1,
                    context.to_relative_display(&path)
                )
                .into());
            }

            if edit.replace_all {
                content = content.replace(&edit.find, &edit.replace);
                applied.push(json!({
                    "edit": index + 1,
                    "replacements": count,
                    "replace_all": true,
                }));
            } else {
                content = content.replacen(&edit.find, &edit.replace, 1);
                applied.push(json!({
                    "edit": index + 1,
                    "replacements": 1,
                    "replace_all": false,
                    "remaining_matches": count.saturating_sub(1),
                }));
            }
        }

        fs::write(&path, content.as_bytes())?;

        Ok(ToolOutput {
            summary: format!(
                "applied {} edits to {}",
                applied.len(),
                context.to_relative_display(&path)
            ),
            content: json!({
                "path": context.to_relative_display(&path),
                "applied": applied,
            }),
        })
    }
}

impl Tool for GrepTextTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep_text".to_string(),
            description: "Search UTF-8 text files in the workspace using plain text or regex.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "path": {"type": "string", "description": "Optional relative root path to search under"},
                    "is_regex": {"type": "boolean"},
                    "max_results": {"type": "integer", "minimum": 1}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: GrepTextInput = serde_json::from_value(input)?;
        let root = context.resolve_path(&input.path)?;
        let matcher = Matcher::new(&input.query, input.is_regex)?;
        let mut matches = Vec::new();

        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| !should_skip(entry.path()))
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let Ok(content) = fs::read_to_string(entry.path()) else {
                continue;
            };

            for (index, line) in content.lines().enumerate() {
                if matcher.is_match(line) {
                    matches.push(json!({
                        "path": context.to_relative_display(entry.path()),
                        "line": index + 1,
                        "text": line,
                    }));

                    if matches.len() >= input.max_results {
                        break;
                    }
                }
            }

            if matches.len() >= input.max_results {
                break;
            }
        }

        Ok(ToolOutput {
            summary: format!("found {} matches", matches.len()),
            content: json!({
                "root": context.to_relative_display(&root),
                "matches": matches,
            }),
        })
    }
}

impl Tool for RunCommandTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_command".to_string(),
            description: "Run a shell command in the workspace and capture stdout/stderr with a timeout.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {"type": "string"},
                    "path": {"type": "string", "description": "Optional relative working directory"},
                    "timeout_seconds": {"type": "integer", "minimum": 1}
                }
            }),
        }
    }

    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let input: RunCommandInput = serde_json::from_value(input)?;
        let cwd = context.resolve_path(&input.path)?;
        let timeout = Duration::from_secs(input.timeout_seconds.max(1));

        let mut child = Command::new("sh")
            .arg("-lc")
            .arg(&input.command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let status = match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("command timed out after {}s", input.timeout_seconds).into());
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

        Ok(ToolOutput {
            summary: format!(
                "command exited with {}",
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            ),
            content: json!({
                "command": input.command,
                "cwd": context.to_relative_display(&cwd),
                "success": status.success(),
                "exit_code": status.code(),
                "stdout": truncate_output(&stdout),
                "stderr": truncate_output(&stderr),
            }),
        })
    }
}

impl Tool for SkillTool {
    fn definition(&self) -> ToolDefinition {
        let description = if self.skills.is_empty() {
            "Load a reusable skill instruction by name. No skills are currently available.".to_string()
        } else {
            format!(
                "Load a reusable skill instruction by name. Available skills:\n{}",
                self.skills.summary_block()
            )
        };

        ToolDefinition {
            name: "skill".to_string(),
            description,
            input_schema: json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "description": "Skill name to load"}
                }
            }),
        }
    }

    fn run(&self, input: Value, _context: &ToolContext) -> AppResult<ToolOutput> {
        let input: SkillInput = serde_json::from_value(input)?;
        let skill = self
            .skills
            .get(&input.name)
            .ok_or_else(|| format!("unknown skill: {}", input.name))?;

        Ok(ToolOutput {
            summary: format!("loaded skill {}", skill.name),
            content: json!({
                "name": skill.name,
                "description": skill.description,
                "license": skill.license,
                "compatibility": skill.compatibility,
                "metadata": skill.metadata,
                "path": skill.path.to_string_lossy(),
                "content": skill.content,
            }),
        })
    }
}

struct Matcher {
    regex: Option<Regex>,
    needle: Option<String>,
}

impl Matcher {
    fn new(query: &str, is_regex: bool) -> AppResult<Self> {
        if is_regex {
            Ok(Self {
                regex: Some(Regex::new(query)?),
                needle: None,
            })
        } else {
            Ok(Self {
                regex: None,
                needle: Some(query.to_ascii_lowercase()),
            })
        }
    }

    fn is_match(&self, line: &str) -> bool {
        if let Some(regex) = &self.regex {
            return regex.is_match(line);
        }

        self.needle
            .as_ref()
            .map(|needle| line.to_ascii_lowercase().contains(needle))
            .unwrap_or(false)
    }
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".git" | "target" | "node_modules")
        )
    })
}

fn truncate_output(text: &str) -> String {
    const LIMIT: usize = 8_000;
    if text.len() <= LIMIT {
        text.to_string()
    } else {
        format!("{}\n...[truncated]", &text[..LIMIT])
    }
}

fn default_max_entries() -> usize {
    100
}

fn default_start_line() -> usize {
    1
}

fn default_max_results() -> usize {
    50
}

fn default_timeout_seconds() -> u64 {
    15
}

fn default_recursive_true() -> bool {
    true
}

fn system_time_to_unix_seconds(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|duration| duration.as_secs())
}
