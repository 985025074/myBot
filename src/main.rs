mod agent;
mod app;
mod config;
mod llm;
mod setup;
mod skills;
mod terminal;
mod tools;
mod ui;

use app::{App, AppResult};
use config::{KeyBindings, LlmConfigStore, ToolPermissionConfig};
use llm::LlmClient;
use setup::{ensure_runtime_setup, load_workspace_env};
use skills::SkillStore;
use std::sync::Arc;
use tools::{CustomToolStore, ToolContext, ToolRegistry};
use app::session::SessionStore;

fn main() -> AppResult<()> {
    let workspace_root = std::env::current_dir()?;
    let runtime_paths = ensure_runtime_setup(&workspace_root)?;
    load_workspace_env(&runtime_paths.env_path)?;

    let keybindings = KeyBindings::load_from_path(&runtime_paths.keybindings_path)?;
    let tool_permissions = ToolPermissionConfig::load_from_path(&runtime_paths.permissions_path)?;
    let llm_profiles = LlmConfigStore::load_from_path(&runtime_paths.llm_config_path)?;
    let llm = LlmClient::new(llm_profiles.active_config()?)?;
    let tool_context = ToolContext::new(&workspace_root)?;
    let skills = Arc::new(SkillStore::discover(&workspace_root, &runtime_paths.runtime_root)?);
    let custom_tools = Arc::new(CustomToolStore::discover(&runtime_paths.runtime_root)?);
    let tools = ToolRegistry::with_extensions(skills.clone(), custom_tools.clone());
    let session_store = SessionStore::new(&runtime_paths.runtime_root)?;
    let initial_session = match session_store.load_current()? {
        Some(session) => session,
        None => {
            let session = session_store.new_session(None, App::default_welcome_messages(&llm));
            session_store.save_session(&session)?;
            session
        }
    };
    let mut terminal = terminal::setup_terminal()?;
    let mut app = App::new(
        keybindings,
        llm,
        llm_profiles,
        runtime_paths.llm_config_path,
        tool_permissions,
        tools,
        skills,
        custom_tools,
        runtime_paths.runtime_scope,
        runtime_paths.runtime_root,
        tool_context,
        session_store,
        initial_session,
    )?;
    let app_result = terminal::run_app(&mut terminal, &mut app);
    terminal::restore_terminal(&mut terminal)?;
    app_result
}

