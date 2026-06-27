//! TUI slash commands — session rename helper.
//!
//! All other commands (clear, compact, new, quit, reasoning, theme, help, todo)
//! are implemented inline in `App::execute_command` to avoid the overhead of a
//! registry + trait dispatch that was never wired up.

use std::sync::Arc;

use crate::panels::sessions::SessionsPanel;
use crate::panels::PanelId;
use crate::App;
use oben_sessions::SessionManager;

/// Execute a session rename with the given new name.
/// Called from `App::handle_key` when a `/rename` command arrives with args.
pub async fn execute_session_rename(app: &mut App, new_name: &str) {
    let agent = {
        let ss = app.shared_state.lock();
        match ss.agent.clone() {
            Some(a) => Arc::clone(&a),
            None => {
                drop(ss);
                app.show_toast(
                    "Rename failed: agent not initialized",
                    ratatui_toaster::ToastType::Error,
                );
                return;
            }
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
    let old_opt = {
        let sm_arc = guard.session_manager_mut();
        let mut sm_guard = sm_arc.lock().await;
        guard
            .context_window_manager()
            .session_id()
            .and_then(|sid| sm_guard.session_mut(&sid))
            .map(|s| s.metadata.title.as_deref().unwrap_or(&s.name).to_string())
    };
    let mut old_title = old_opt.unwrap_or_else(|| "unnamed".to_string());

    // Fall back to session name/title if no title was set yet
    if old_title == "unnamed" {
        let sm_arc = guard.session_manager();
        let sm_guard = sm_arc.lock().await;
        let old_opt = guard
            .context_window_manager()
            .session_id()
            .and_then(|sid| sm_guard.session(&sid))
            .map(|s| s.metadata.title.as_deref().unwrap_or(&s.name).to_string());
        old_title = old_opt.unwrap_or_else(|| "unnamed".to_string());
    }

    guard
        .session_manager_mut()
        .lock()
        .await
        .set_title(new_name)
        .map_err(|e| e.to_string())?;
    Ok(old_title)
}
