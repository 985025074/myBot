use std::{env, fs, path::{Path, PathBuf}};

use crate::{
    app::AppResult,
    config::LlmConfigStore,
};

const DEFAULT_PERMISSIONS_TOML: &str = r#"default = "allow"

[tools.run_command]
"*" = "ask"
"git *" = "allow"

[tools.edit]
"*" = "ask"
"src/**" = "allow"
"#;

const DEFAULT_KEYBINDINGS_TOML: &str = r#"[keybindings]
quit = ["q"]
clear_or_exit = ["ctrl+c"]
scroll_up = ["pageup"]
scroll_down = ["pagedown"]
navigate_up = ["up"]
navigate_down = ["down"]
move_left = ["left"]
move_right = ["right"]
move_line_start = ["home"]
move_line_end = ["end"]
delete_backward = ["backspace"]
delete_forward = ["delete"]
insert_newline = ["alt+enter"]
submit_input = ["enter"]
autocomplete_command = ["tab"]
clear_input = ["esc"]
open_config = ["f2"]
save_config = ["ctrl+s"]
close_config = ["esc"]
config_next_field = ["tab"]
config_previous_field = ["shift+tab"]
toggle_thinking = ["f3"]
toggle_tool_details = ["f4"]
approve_tool = ["y", "enter"]
always_allow_tool = ["a"]
always_deny_tool = ["d"]
reject_tool = ["n", "esc"]
"#;

pub struct RuntimePaths {
    pub runtime_root: PathBuf,
    pub runtime_scope: RuntimeScope,
    pub keybindings_path: PathBuf,
    pub llm_config_path: PathBuf,
    pub permissions_path: PathBuf,
    pub env_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeScope {
    Workspace,
    Home,
}

impl RuntimeScope {
    pub fn detect() -> Self {
        match env::var("MYBOT_RUNTIME_SCOPE") {
            Ok(value) if value.eq_ignore_ascii_case("home") => Self::Home,
            Ok(value) if value.eq_ignore_ascii_case("workspace") => Self::Workspace,
            _ if cfg!(debug_assertions) => Self::Workspace,
            _ => Self::Home,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Home => "home",
        }
    }
}

pub fn ensure_runtime_setup(workspace_root: &Path) -> AppResult<RuntimePaths> {
    let runtime_scope = RuntimeScope::detect();
    let runtime_root = runtime_root(workspace_root, runtime_scope)?;
    ensure_scaffold_dirs(&runtime_root)?;
    ensure_config_files(&runtime_root)?;

    Ok(RuntimePaths {
        runtime_scope,
        keybindings_path: runtime_root.join("config/keybindings.toml"),
        llm_config_path: runtime_root.join("config/llm.toml"),
        permissions_path: runtime_root.join("config/permissions.toml"),
        env_path: runtime_root.join(".env"),
        runtime_root,
    })
}

pub fn load_workspace_env(path: &Path) -> AppResult<()> {
    if path.exists() {
        dotenvy::from_path(path)?;
    }

    Ok(())
}

fn runtime_root(workspace_root: &Path, scope: RuntimeScope) -> AppResult<PathBuf> {
    match scope {
        RuntimeScope::Workspace => Ok(workspace_root.join(".mybot")),
        RuntimeScope::Home => {
            let home = env::var_os("HOME").ok_or("HOME is not set")?;
            Ok(PathBuf::from(home).join(".mybot"))
        }
    }
}

fn ensure_config_files(runtime_root: &Path) -> AppResult<()> {
    let config_dir = runtime_root.join("config");

    write_if_missing(
        &config_dir.join("llm.toml"),
        &toml::to_string_pretty(&LlmConfigStore::default())?,
    )?;
    write_if_missing(
        &config_dir.join("permissions.toml"),
        DEFAULT_PERMISSIONS_TOML,
    )?;
    write_if_missing(
        &config_dir.join("keybindings.toml"),
        DEFAULT_KEYBINDINGS_TOML,
    )?;
    write_if_missing(
        &runtime_root.join(".env"),
        &env_template([
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "BAILIAN_CODING_PLAN_API_KEY",
        ]),
    )?;

    Ok(())
}

fn ensure_scaffold_dirs(root: &Path) -> AppResult<()> {
    fs::create_dir_all(root.join("config"))?;
    fs::create_dir_all(root.join("sessions"))?;
    fs::create_dir_all(root.join("skills"))?;
    fs::create_dir_all(root.join("tools"))?;
    Ok(())
}

fn write_if_missing(path: &Path, content: &str) -> AppResult<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, content)?;
    Ok(())
}

fn env_template<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut lines = vec![
        "# mybot local secrets".to_string(),
        "# Keep this file out of version control.".to_string(),
    ];

    let mut wrote_name = false;
    for name in names {
        if !wrote_name {
            lines.push(String::new());
            wrote_name = true;
        }
        lines.push(format!("# {name}=<your-api-key>"));
    }

    lines.push(String::new());
    lines.join("\n")
}
