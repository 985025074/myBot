mod builtin;
mod custom;

use std::{collections::HashMap, fs, path::{Path, PathBuf}, sync::Arc};

use serde::Serialize;
use serde_json::Value;

use crate::app::AppResult;
use crate::skills::SkillStore;

pub use builtin::register_builtin_tools;
pub use custom::{CustomToolStore, register_custom_tools};

#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolOutput {
    pub summary: String,
    pub content: Value,
}

#[derive(Debug, Clone)]
pub struct ToolPermissionDescriptor {
    pub tool: String,
    pub scopes: Vec<String>,
    pub subjects: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceUndoSnapshot {
    pub tool: String,
    pub summary: String,
    pub states: Vec<PathStateSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PathStateSnapshot {
    pub path: String,
    pub state: FsNodeSnapshot,
}

#[derive(Debug, Clone)]
pub enum FsNodeSnapshot {
    Missing,
    File(Vec<u8>),
    Directory(Vec<DirectoryEntrySnapshot>),
}

#[derive(Debug, Clone)]
pub struct DirectoryEntrySnapshot {
    pub name: String,
    pub state: FsNodeSnapshot,
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    workspace_root: PathBuf,
}

pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn run(&self, input: Value, context: &ToolContext) -> AppResult<ToolOutput>;
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

impl ToolContext {
    pub fn new(workspace_root: impl Into<PathBuf>) -> AppResult<Self> {
        let workspace_root = workspace_root.into().canonicalize()?;
        Ok(Self { workspace_root })
    }

    pub fn resolve_path(&self, path: &str) -> AppResult<PathBuf> {
        let requested = if path.trim().is_empty() {
            self.workspace_root.clone()
        } else {
            self.workspace_root.join(path)
        };

        self.ensure_within_workspace(&requested)
    }

    pub fn prepare_path(&self, path: &str) -> AppResult<PathBuf> {
        let requested = if path.trim().is_empty() {
            self.workspace_root.clone()
        } else {
            self.workspace_root.join(path)
        };

        self.ensure_within_workspace(&requested)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn ensure_within_workspace(&self, path: &Path) -> AppResult<PathBuf> {
        let resolved = if path.exists() {
            path.canonicalize()?
        } else {
            let mut missing_parts = Vec::new();
            let mut cursor = path;

            while !cursor.exists() {
                let file_name = cursor
                    .file_name()
                    .ok_or_else(|| format!("invalid path: {}", path.to_string_lossy()))?;
                missing_parts.push(file_name.to_owned());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("invalid path: {}", path.to_string_lossy()))?;
            }

            let mut resolved = cursor.canonicalize()?;
            for part in missing_parts.iter().rev() {
                resolved.push(part);
            }
            resolved
        };

        if !resolved.starts_with(&self.workspace_root) {
            return Err(format!("path escapes workspace: {}", path.to_string_lossy()).into());
        }

        Ok(resolved)
    }

    pub fn to_relative_display(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace_root)
            .map(|relative| {
                let text = relative.to_string_lossy().replace('\\', "/");
                if text.is_empty() {
                    ".".to_string()
                } else {
                    text
                }
            })
            .unwrap_or_else(|_| path.to_string_lossy().into_owned())
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn with_builtins() -> Self {
        Self::with_extensions(
            Arc::new(SkillStore::default()),
            Arc::new(CustomToolStore::default()),
        )
    }

    #[allow(dead_code)]
    pub fn with_skills(skills: Arc<SkillStore>) -> Self {
        Self::with_extensions(skills, Arc::new(CustomToolStore::default()))
    }

    pub fn with_extensions(
        skills: Arc<SkillStore>,
        custom_tools: Arc<CustomToolStore>,
    ) -> Self {
        let mut registry = Self::new();
        register_builtin_tools(&mut registry, skills);
        register_custom_tools(&mut registry, custom_tools);
        registry
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let definition = tool.definition();
        self.tools.insert(definition.name.clone(), Arc::new(tool));
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self
            .tools
            .values()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    pub fn execute(&self, name: &str, input: Value, context: &ToolContext) -> AppResult<ToolOutput> {
        let Some(tool) = self.tools.get(name) else {
            return Err(format!("unknown tool: {name}").into());
        };

        tool.run(input, context)
    }

    pub fn permission_descriptor(
        &self,
        name: &str,
        input: &Value,
        context: &ToolContext,
    ) -> ToolPermissionDescriptor {
        match name {
            "run_command" => {
                let command = string_field(input, "command").unwrap_or_default();
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string(), "command".to_string()],
                    subjects: vec![command.clone()],
                    summary: command,
                }
            }
            "write_file" | "apply_patch" | "make_directory" | "delete_path" => {
                let path = path_subject(input, "path", context);
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string(), "edit".to_string()],
                    subjects: vec![path.clone()],
                    summary: path,
                }
            }
            "move_path" => {
                let source = path_subject(input, "source", context);
                let destination = path_subject(input, "destination", context);
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string(), "edit".to_string()],
                    subjects: vec![source.clone(), destination.clone()],
                    summary: format!("{source} -> {destination}"),
                }
            }
            "read_file" | "file_stat" => {
                let path = path_subject(input, "path", context);
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string(), "read".to_string()],
                    subjects: vec![path.clone()],
                    summary: path,
                }
            }
            "list_files" | "glob_files" | "grep_text" => {
                let subject = string_field(input, "path")
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| display_path(context, &value))
                    .or_else(|| string_field(input, "pattern"))
                    .or_else(|| string_field(input, "query"))
                    .unwrap_or_else(|| ".".to_string());
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string()],
                    subjects: vec![subject.clone()],
                    summary: subject,
                }
            }
            "skill" => {
                let skill = string_field(input, "name").unwrap_or_default();
                ToolPermissionDescriptor {
                    tool: name.to_string(),
                    scopes: vec![name.to_string(), "skill".to_string()],
                    subjects: vec![skill.clone()],
                    summary: skill,
                }
            }
            _ => ToolPermissionDescriptor {
                tool: name.to_string(),
                scopes: vec![name.to_string()],
                subjects: vec![input.to_string()],
                summary: input.to_string(),
            },
        }
    }

    pub fn capture_undo_snapshot(
        &self,
        name: &str,
        input: &Value,
        context: &ToolContext,
    ) -> AppResult<Option<WorkspaceUndoSnapshot>> {
        let snapshot = match name {
            "write_file" | "apply_patch" | "make_directory" | "delete_path" => {
                let path = path_subject(input, "path", context);
                Some(WorkspaceUndoSnapshot {
                    tool: name.to_string(),
                    summary: path.clone(),
                    states: vec![PathStateSnapshot {
                        path: path.clone(),
                        state: snapshot_path(context, &path)?,
                    }],
                })
            }
            "move_path" => {
                let source = path_subject(input, "source", context);
                let destination = path_subject(input, "destination", context);
                Some(WorkspaceUndoSnapshot {
                    tool: name.to_string(),
                    summary: format!("{source} -> {destination}"),
                    states: vec![
                        PathStateSnapshot {
                            path: source.clone(),
                            state: snapshot_path(context, &source)?,
                        },
                        PathStateSnapshot {
                            path: destination.clone(),
                            state: snapshot_path(context, &destination)?,
                        },
                    ],
                })
            }
            _ => None,
        };

        Ok(snapshot)
    }
}

pub fn apply_workspace_undo_snapshot(
    snapshot: &WorkspaceUndoSnapshot,
    context: &ToolContext,
) -> AppResult<()> {
    for state in snapshot.states.iter().rev() {
        let path = context.prepare_path(&state.path)?;
        restore_path_snapshot(&path, &state.state)?;
    }

    Ok(())
}

fn string_field(input: &Value, key: &str) -> Option<String> {
    input.get(key)?.as_str().map(ToString::to_string)
}

fn path_subject(input: &Value, key: &str, context: &ToolContext) -> String {
    string_field(input, key)
        .map(|value| display_path(context, &value))
        .unwrap_or_else(|| ".".to_string())
}

fn display_path(context: &ToolContext, raw: &str) -> String {
    context
        .prepare_path(raw)
        .map(|path| context.to_relative_display(&path))
        .unwrap_or_else(|_| raw.to_string())
}

fn snapshot_path(context: &ToolContext, raw: &str) -> AppResult<FsNodeSnapshot> {
    let path = context.prepare_path(raw)?;
    snapshot_node(&path)
}

fn snapshot_node(path: &Path) -> AppResult<FsNodeSnapshot> {
    if !path.exists() {
        return Ok(FsNodeSnapshot::Missing);
    }

    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        return Ok(FsNodeSnapshot::File(fs::read(path)?));
    }

    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let state = snapshot_node(&entry.path())?;
                Ok(DirectoryEntrySnapshot { name, state })
            })
            .collect::<AppResult<Vec<_>>>()?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        return Ok(FsNodeSnapshot::Directory(entries));
    }

    Ok(FsNodeSnapshot::Missing)
}

fn restore_path_snapshot(path: &Path, snapshot: &FsNodeSnapshot) -> AppResult<()> {
    match snapshot {
        FsNodeSnapshot::Missing => {
            remove_existing_path(path)?;
        }
        FsNodeSnapshot::File(bytes) => {
            remove_existing_path(path)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
        FsNodeSnapshot::Directory(entries) => {
            remove_existing_path(path)?;
            fs::create_dir_all(path)?;
            for entry in entries {
                restore_path_snapshot(&path.join(&entry.name), &entry.state)?;
            }
        }
    }

    Ok(())
}

fn remove_existing_path(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }

    Ok(())
}
