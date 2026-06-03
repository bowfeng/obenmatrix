//! TUI slash commands — session rename helper.
//!
//! All other commands (clear, compact, new, quit, reasoning, theme, help, todo)
//! are implemented inline in `App::execute_command` to avoid the overhead of a
//! registry + trait dispatch that was never wired up.

use std::sync::Arc;

use crate::panels::sessions::SessionsPanel;
use crate::panels::PanelId;
use crate::App;

/// Execute a session rename with the given new name.
/// Called from `App::handle_key` when a `/rename` command arrives with args.
pub async fn execute_session_rename(app: &mut App, new_name: &str) {
    let agent = match &app.agent {
        Some(a) => Arc::clone(a),
        None => {
            app.show_toast(
                "Rename failed: agent not initialized",
                ratatui_toaster::ToastType::Error,
            );
            return;
        }
    };

    let result = rename_inner(agent, new_name).await;

    match result {
        Ok(old_name) => {
            if let Some(chat) = app.get_chat_mut() {
                chat.session_name = Some(new_name.to_string());
            }
            // Update the SessionsPanel's cached list so the renamed name is
            // visible immediately when switching to the sessions panel.
            if let Some(sessions) = app
                .panels
                .get_mut(&PanelId::Sessions)
                .and_then(|p| p.downcast_mut::<SessionsPanel>())
            {
                sessions.refresh_display_name(&old_name, new_name);
            }
            app.show_toast(
                format!("Session renamed: {old_name} \u{2192} {new_name}"),
                ratatui_toaster::ToastType::Success,
            );
        }
        Err(e) => {
            app.show_toast(e, ratatui_toaster::ToastType::Error);
        }
    }
}

async fn rename_inner(
    agent: Arc<tokio::sync::Mutex<oben_agent::Agent>>,
    new_name: &str,
) -> Result<String, String> {
    let mut guard = agent.lock().await;
    let mut old_title = guard
        .session_manager_mut()
        .lock()
        .await
        .active_session()
        .and_then(|s| s.metadata.title.clone())
        .map(|t| t.clone())
        .unwrap_or_else(|| "unnamed".to_string());

    // Fall back to session name/title if no title was set yet
    if old_title == "unnamed" {
        old_title = guard
            .session_manager()
            .lock()
            .await
            .active_session()
            .map(|s| s.metadata.title.as_deref().unwrap_or(&s.name).to_string())
            .unwrap_or_else(|| "unnamed".to_string());
    }

    guard
        .session_manager_mut()
        .lock()
        .await
        .set_title(new_name)
        .map_err(|e| e.to_string())?;
    Ok(old_title)
}

fn get_session_display_name(sm: &oben_sessions::SessionManager) -> String {
    sm.active_session()
        .map(|s| s.metadata.title.as_deref().unwrap_or(&s.name).to_string())
        .unwrap_or_else(|| "unnamed".to_string())
}
