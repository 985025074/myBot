mod builtin;

use std::{collections::HashMap, path::{Path, PathBuf}, sync::Arc};

use serde::Serialize;
use serde_json::Value;

use crate::app::AppResult;

pub use builtin::register_builtin_tools;

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

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        register_builtin_tools(&mut registry);
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
}
