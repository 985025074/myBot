mod agent;
mod app;
mod config;
mod llm;
mod terminal;
mod tools;
mod ui;

use app::{App, AppResult};
use config::{KeyBindings, LlmConfigStore, ToolPermissionConfig};
use llm::LlmClient;
use std::path::Path;
use tools::{ToolContext, ToolRegistry};

fn main() -> AppResult<()> {
    let keybindings = KeyBindings::load_from_path(Path::new("config/keybindings.toml"))?;
    let llm_config_path = Path::new("config/llm.toml");
    let tool_permissions = ToolPermissionConfig::load_from_path(Path::new("config/permissions.toml"))?;
    let llm_profiles = LlmConfigStore::load_from_path(llm_config_path)?;
    let llm = LlmClient::new(llm_profiles.active_config()?)?;
    let tool_context = ToolContext::new(std::env::current_dir()?)?;
    let tools = ToolRegistry::with_builtins();
    let mut terminal = terminal::setup_terminal()?;
    let mut app = App::new(
        keybindings,
        llm,
        llm_profiles,
        llm_config_path.to_path_buf(),
        tool_permissions,
        tools,
        tool_context,
    );
    let app_result = terminal::run_app(&mut terminal, &mut app);
    terminal::restore_terminal(&mut terminal)?;
    app_result
}

