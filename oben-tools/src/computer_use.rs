//! Computer Use tool — Cross-platform desktop control.
//!
//! Platform backends:
//! - macOS: cua-driver (via `cua-driver call <tool> <json-args>`)
//! - Linux: xautomation (via `xdo click`, `xdo type`, etc.)
//! - Windows: pyautogui (via `python -m pyautogui <action>`)

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::process::{Command, Stdio};
use std::sync::{OnceLock};
use tracing::debug;

use oben_models::{ToolMeta, ToolParameters, ToolResult};

use super::registry::{Tool, ToolCall, ToolRegistry};

// ===========================================================================
// Platform Detection and Configuration
// ===========================================================================

fn detect_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

fn get_platform_backend() -> &'static str {
    let platform = detect_platform();
    // For future backend override support
    let _ = std::env::var("COMPUTER_USE_BACKEND");
    platform
}

/// Check if the required backend is available on this system.
pub fn check_computer_use_requirements() -> bool {
    let backend = get_platform_backend();
    match backend {
        "macos" => {
            Command::new("cua-driver").arg("--version").output().is_ok()
        }
        "linux" => {
            Command::new("xdo").arg("--version").output().is_ok()
                && Command::new("xdotool").arg("--version").output().is_ok()
        }
        "windows" => {
            // Check if Python is available and pyautogui is installed
            let python_check = Command::new("python").arg("-m").arg("pyautogui").arg("--version").output().is_ok()
                || Command::new("python3").arg("-m").arg("pyautogui").arg("--version").output().is_ok();
            python_check
        }
        _ => false,
    }
}

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
                    "list_apps", "focus_app", "press_key"
                ],
                "description": "Which action to perform. capture is free (no side effects). Use set_value for select/popup elements and sliders."
            },
            "mode": {
                "type": "string",
                "enum": ["som", "vision", "ax"],
                "description": "Capture mode: som (screenshot+numbered overlays, default), vision (plain screenshot), ax (accessibility tree only). macOS only."
            },
            "app": {
                "type": "string",
                "description": "Optional: limit capture/action to a specific app (by name or bundle ID)."
            },
            "max_elements": {
                "type": "integer",
                "minimum": 1,
                "maximum": 1000,
                "description": "Optional cap on AX elements returned by capture. Default 100. macOS only."
            },
            "element": {
                "type": "integer",
                "description": "The 1-based SOM index returned by the last capture(mode='som'). Preferred over raw coordinates. macOS only."
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
                "items": {"type": "string", "enum": ["cmd", "shift", "option", "alt", "ctrl", "fn", "super", "hyper"]},
                "description": "Modifier keys held during the action."
            },
            "from_element": {"type": "integer", "description": "Source element index for drag. macOS only."},
            "to_element": {"type": "integer", "description": "Target element index for drag. macOS only."},
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
                "description": "If true, take a follow-up capture after the action. macOS only."
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
    BLOCKED_KEYS.get_or_init(|| vec![
        "cmd+shift+q".into(), 
        "ctrl+cmd+q".into(),
        "alt+f4".into(), 
        "ctrl+alt+del".into(),
    ])
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
// macOS Backend — cua-driver
// ===========================================================================

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

fn extract_result(raw: &str) -> (String, Vec<String>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();

    if let Ok(obj) = serde_json::from_str::<Value>(raw) {
        extract_inner(&obj, &mut text_parts, &mut images);
    } else {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            text_parts.push(trimmed.to_string());
        }
    }

    let text = text_parts.join("\n").trim().to_string();
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
        if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
            if !t.is_empty() {
                texts.push(t.to_string());
            }
        }
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
        if let Some(arr) = obj.get("content").and_then(|v| v.as_array()) {
            for item in arr {
                extract_inner(item, texts, imgs);
            }
        }
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

fn resolve_target_windows_macos(app: Option<&str>) -> Option<(i64, i64)> {
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
// Linux Backend — xautomation (xdotool + xdo)
// ===========================================================================

fn call_xdo_command(args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("xdotool");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    
    debug!("xdotool command: {:?}", args);
    
    let output = cmd.output()?;
    
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow::anyhow!("xdotool failed: {err}"));
    }
    
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn get_current_window_linux() -> Result<String> {
    call_xdo_command(&["getwindowfocus"])
}

fn call_xdo_click(button: &str) -> Result<()> {
    let button_num = match button {
        "left" => "1",
        "right" => "3",
        "middle" => "2",
        _ => "1",
    };
    call_xdo_command(&["click", button_num])?;
    Ok(())
}

fn call_xdo_type(text: &str) -> Result<()> {
    call_xdo_command(&["type", "--", text])?;
    Ok(())
}

fn call_xdo_key(key: &str) -> Result<()> {
    call_xdo_command(&["key", key])?;
    Ok(())
}

fn call_xdo_scroll(direction: &str, amount: i64) -> Result<()> {
    let (dx, dy) = match direction {
        "up" => (0, -amount),
        "down" => (0, amount),
        "left" => (-amount, 0),
        "right" => (amount, 0),
        _ => (0, -amount),
    };
    
    // Scroll using xdotool
    let scroll_args = if dy != 0 {
        format!("--delay 50 {} {}", dy, -dy)
    } else {
        format!("--delay 50 {} {}", dx, -dx)
    };
    
    call_xdo_command(&["mousemove_relative", "--", &scroll_args])?;
    Ok(())
}

fn call_xdo_move(x: i64, y: i64) -> Result<()> {
    call_xdo_command(&["mousemove", "--", &x.to_string(), &y.to_string()])?;
    Ok(())
}

// ===========================================================================
// Windows Backend — pyautogui
// ===========================================================================

fn call_pyautogui_python_module(action: &str, args: &[&str]) -> Result<String> {
    // Try both python and python3
    let python_exe = if Command::new("python").arg("-c").arg("import pyautogui").output().is_ok() {
        "python"
    } else {
        "python3"
    };
    
    let mut cmd = Command::new(python_exe);
    cmd.arg("-m").arg("pyautogui").arg(action).args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    
    debug!("pyautogui command: {:?}", cmd);
    
    let output = cmd.output()?;
    
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow::anyhow!("pyautogui failed: {err}"));
    }
    
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn call_pyautogui_click(button: &str) -> Result<()> {
    let button_arg = match button {
        "left" => "left",
        "right" => "right",
        "middle" => "middle",
        _ => "left",
    };
    call_pyautogui_python_module("click", &[button_arg])?;
    Ok(())
}

fn call_pyautogui_type(text: &str) -> Result<()> {
    // Escape special characters for pyautogui
    let escaped_text = text
        .replace("%", "%%")
        .replace("{", "{{")
        .replace("}", "}}");
    
    call_pyautogui_python_module("typewrite", &[&escaped_text])?;
    Ok(())
}

fn call_pyautogui_key(key: &str) -> Result<()> {
    call_pyautogui_python_module("press", &[key])?;
    Ok(())
}

fn call_pyautogui_scroll(amount: i64) -> Result<()> {
    call_pyautogui_python_module("scroll", &[&amount.to_string()])?;
    Ok(())
}

fn call_pyautogui_move(x: i64, y: i64) -> Result<()> {
    call_pyautogui_python_module("moveTo", &[&x.to_string(), &y.to_string()])?;
    Ok(())
}



// ===========================================================================
// Platform-specific capture
// ===========================================================================

fn capture_macos(args: &Value) -> Result<ToolResult> {
    let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("som");
    let _max_elements = args
        .get("max_elements")
        .and_then(|v| v.as_i64())
        .unwrap_or(100);
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;

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
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output,
        error: None,
    })
}

fn capture_linux(args: &Value) -> Result<ToolResult> {
    let _app = args.get("app").and_then(|v| v.as_str());
    
    // Get current window
    let window = get_current_window_linux()
        .map_err(|e| anyhow::anyhow!("Failed to get current window: {e}"))?;
    
    // Take screenshot using spectacle or import
    let screenshot_output = Command::new("import")
        .arg("-window")
        .arg(&window)
        .arg("png:-")
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to take screenshot: {e}"))?;
    
    let base64_img = base64::encode(&screenshot_output.stdout);
    
    let output = format!("Screenshot captured for window {window}\nDATA:image/png;base64,{base64_img}");
    
    Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output,
        error: None,
    })
}

fn capture_windows(args: &Value) -> Result<ToolResult> {
    // For Windows, we'll use pyautogui to get screenshot info
    // Note: True screenshot capture on Windows requires additional libs
    // This is a simplified version that reports the active window
    
    let output = "Windows screenshot capture requires pyautogui + PIL\n\
                  For full functionality, install: pip install pyautogui pillow";
    
   Ok(ToolResult {
        call_id: args.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        output: output.to_string(),
        error: None,
    })
}

// ===========================================================================
// Platform-specific pointer actions
// ===========================================================================

fn handle_pointer_macos(args: &Value, action: &str, call_id: String) -> Result<ToolResult> {
    let mut args_map = serde_json::Map::new();
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;
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

fn handle_pointer_linux(args: &Value, action: &str, call_id: String) -> Result<ToolResult> {
    let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");
    
    // For Linux, we use xdotool for basic actions
    let _element = args.get("element").and_then(|v| v.as_i64());
    let coord = args.get("coordinate").and_then(|v| v.as_array());
    
    // Handle click action
    if action == "click" || action == "double_click" || action == "right_click" {
        if let Some(coord) = coord {
            if let (Some(x), Some(y)) = (
                coord.get(0).and_then(|v| v.as_i64()),
                coord.get(1).and_then(|v| v.as_i64()),
            ) {
                call_xdo_move(x, y)?;
            }
        }
        call_xdo_click(button)?;
    }
    
    // Handle double_click
    if action == "double_click" {
        // xdotool doesn't have double_click, so we click twice
        call_xdo_click(button)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        call_xdo_click(button)?;
    }
    
    Ok(ToolResult {
        call_id,
        output: format!("Linux {action} executed with button={button}"),
        error: None,
    })
}

fn handle_pointer_windows(args: &Value, action: &str, call_id: String) -> Result<ToolResult> {
    let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");
    let _element = args.get("element").and_then(|v| v.as_i64());
    let coord = args.get("coordinate").and_then(|v| v.as_array());
    
    if let Some(coord) = coord {
        if let (Some(x), Some(y)) = (
            coord.get(0).and_then(|v| v.as_i64()),
            coord.get(1).and_then(|v| v.as_i64()),
        ) {
            call_pyautogui_move(x, y)?;
        }
    }
    
    if action == "click" || action == "double_click" || action == "right_click" {
        call_pyautogui_click(button)?;
    }
    
    if action == "double_click" {
        call_pyautogui_click(button)?;
    }
    
    Ok(ToolResult {
        call_id,
        output: format!("Windows {action} executed with button={button}"),
        error: None,
    })
}

// ===========================================================================
// Platform-specific type actions
// ===========================================================================

fn handle_type_macos(args: &Value, call_id: String) -> Result<ToolResult> {
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
    let (pid, _) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;

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

fn handle_type_linux(args: &Value, call_id: String) -> Result<ToolResult> {
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

    call_xdo_type(text)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Typed text on Linux"),
        error: None,
    })
}

fn handle_type_windows(args: &Value, call_id: String) -> Result<ToolResult> {
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

    call_pyautogui_type(text)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Typed text on Windows"),
        error: None,
    })
}

// ===========================================================================
// Platform-specific key actions
// ===========================================================================

fn handle_key_macos(args: &Value, call_id: String) -> Result<ToolResult> {
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
    let (pid, _) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;

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

fn handle_key_linux(args: &Value, call_id: String) -> Result<ToolResult> {
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

    call_xdo_key(keys)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Pressed keys on Linux: {keys}"),
        error: None,
    })
}

fn handle_key_windows(args: &Value, call_id: String) -> Result<ToolResult> {
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

    // Convert key format for pyautogui (e.g., "cmd+s" -> "super+s")
    let pyautogui_keys = keys.replace("cmd", "super").replace("option", "alt");
    call_pyautogui_key(&pyautogui_keys)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Pressed keys on Windows: {keys}"),
        error: None,
    })
}

// ===========================================================================
// Platform-specific drag actions
// ===========================================================================

fn handle_drag_macos(args: &Value, call_id: String) -> Result<ToolResult> {
    let mut args_map = serde_json::Map::new();
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;
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

fn handle_drag_linux(args: &Value, call_id: String) -> Result<ToolResult> {
    let from_coord = args.get("from_coordinate").and_then(|v| v.as_array());
    let to_coord = args.get("to_coordinate").and_then(|v| v.as_array());
    
    if let (Some(from), Some(to)) = (from_coord, to_coord) {
        if let (Some(fx), Some(fy)) = (
            from.get(0).and_then(|v| v.as_i64()),
            from.get(1).and_then(|v| v.as_i64()),
        ) {
            call_xdo_move(fx, fy)?;
        }
        if let (Some(tx), Some(ty)) = (
            to.get(0).and_then(|v| v.as_i64()),
            to.get(1).and_then(|v| v.as_i64()),
        ) {
            call_xdo_command(&["mousedown", "1"])?;
            call_xdo_move(tx, ty)?;
            call_xdo_command(&["mouseup", "1"])?;
        }
    }
    
    Ok(ToolResult {
        call_id,
        output: "Linux drag executed".to_string(),
        error: None,
    })
}

fn handle_drag_windows(args: &Value, call_id: String) -> Result<ToolResult> {
    let from_coord = args.get("from_coordinate").and_then(|v| v.as_array());
    let to_coord = args.get("to_coordinate").and_then(|v| v.as_array());
    
    if let (Some(from), Some(to)) = (from_coord, to_coord) {
        if let (Some(fx), Some(fy)) = (
            from.get(0).and_then(|v| v.as_i64()),
            from.get(1).and_then(|v| v.as_i64()),
        ) {
            call_pyautogui_move(fx, fy)?;
        }
        pyautogui_mouse_down("left")?;
        if let (Some(tx), Some(ty)) = (
            to.get(0).and_then(|v| v.as_i64()),
            to.get(1).and_then(|v| v.as_i64()),
        ) {
            call_pyautogui_move(tx, ty)?;
        }
        pyautogui_mouse_up("left")?;
    }
    
    Ok(ToolResult {
        call_id,
        output: "Windows drag executed".to_string(),
        error: None,
    })
}

fn pyautogui_mouse_down(button: &str) -> Result<()> {
    call_pyautogui_python_module("mouseDown", &[button])?;
    Ok(())
}

fn pyautogui_mouse_up(button: &str) -> Result<()> {
    call_pyautogui_python_module("mouseUp", &[button])?;
    Ok(())
}

// ===========================================================================
// Platform-specific scroll actions
// ===========================================================================

fn handle_scroll_macos(args: &Value, call_id: String) -> Result<ToolResult> {
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("scroll requires 'direction' argument"))?;
    let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;

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

fn handle_scroll_linux(args: &Value, call_id: String) -> Result<ToolResult> {
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("scroll requires 'direction' argument"))?;
    let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);

    call_xdo_scroll(direction, amount)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Linux scroll executed: {direction} {amount}"),
        error: None,
    })
}

fn handle_scroll_windows(args: &Value, call_id: String) -> Result<ToolResult> {
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("scroll requires 'direction' argument"))?;
    let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);

    let scroll_amount = match direction {
        "up" => amount,
        "down" => -amount,
        "left" => -amount,
        "right" => amount,
        _ => -amount,
    };

    call_pyautogui_scroll(scroll_amount)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Windows scroll executed: {direction} {amount}"),
        error: None,
    })
}

// ===========================================================================
// Platform-specific wait actions
// ===========================================================================

async fn handle_wait_macos(args: &Value, call_id: String) -> Result<ToolResult> {
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

fn handle_wait_linux(args: &Value, call_id: String) -> Result<ToolResult> {
    let seconds = args
        .get("seconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .max(0.0)
        .min(30.0);
    
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    
    Ok(ToolResult {
        call_id,
        output: format!("Waited {seconds:.2}s on Linux"),
        error: None,
    })
}

fn handle_wait_windows(args: &Value, call_id: String) -> Result<ToolResult> {
    let seconds = args
        .get("seconds")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .max(0.0)
        .min(30.0);
    
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    
    Ok(ToolResult {
        call_id,
        output: format!("Waited {seconds:.2}s on Windows"),
        error: None,
    })
}

// ===========================================================================
// Platform-specific list_apps actions
// ===========================================================================

fn handle_list_apps_macos(call_id: String) -> Result<ToolResult> {
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

fn handle_list_apps_linux(call_id: String) -> Result<ToolResult> {
    // On Linux, we can use `wmctrl -l` or `xdotool search --onlyvisible --class .`
    let output = call_xdo_command(&["search", "--onlyvisible", "--class", "."])
        .unwrap_or_else(|_| "xdotool not fully available".to_string());
    
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

fn handle_list_apps_windows(call_id: String) -> Result<ToolResult> {
    // On Windows, we can use PowerShell to list windows
    let output = Command::new("powershell")
        .arg("-Command")
        .arg("Get-Process | Where-Object {$_.MainWindowTitle} | Select-Object ProcessName,MainWindowTitle")
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to list windows: {e}"))?;
    
    let text = String::from_utf8_lossy(&output.stdout);
    
    Ok(ToolResult {
        call_id,
        output: text.to_string(),
        error: None,
    })
}

// ===========================================================================
// Platform-specific focus_app actions
// ===========================================================================

fn handle_focus_app_macos(args: &Value, call_id: String) -> Result<ToolResult> {
    let app = args
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("focus_app requires 'app' argument"))?;

    let (pid, window_id) = resolve_target_windows_macos(Some(app))
        .ok_or_else(|| anyhow::anyhow!("No on-screen window found for app '{app}'"))?;

    let output = format!("Targeted window: pid={pid}, window_id={window_id} (app={app})");
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

fn handle_focus_app_linux(args: &Value, call_id: String) -> Result<ToolResult> {
    let app = args
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("focus_app requires 'app' argument"))?;

    // Use xdotool to find and focus window
    let window_id = call_xdo_command(&["search", "--name", app])
        .map_err(|e| anyhow::anyhow!("Failed to find window for app '{app}': {e}"))?;
    
    call_xdo_command(&["windowactivate", &window_id])?;
    
    let output = format!("Focused window: {window_id} (app={app})");
    Ok(ToolResult {
        call_id,
        output,
        error: None,
    })
}

fn handle_focus_app_windows(args: &Value, call_id: String) -> Result<ToolResult> {
    let app = args
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("focus_app requires 'app' argument"))?;

    // Use PowerShell to find and activate window
    let script = format!(
        r#"
        $app = "{app}"
        $process = Get-Process | Where-Object {{ $_.MainWindowTitle -match $app }} | Select-Object -First 1
        if ($process) {{
            $windowHandle = $process.MainWindowHandle
            if ($windowHandle -ne [IntPtr]::Zero) {{
                $null = Show-Window -Handle $windowHandle -Active
                Write-Output "Focused window: $($process.ProcessName) (Handle: $windowHandle)"
            }} else {{
                Write-Output "No visible window found for $app"
            }}
        }} else {{
            Write-Output "Process not found: $app"
        }}
        "#
    );

    let output = Command::new("powershell")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to focus window: {e}"))?;
    
    let text = String::from_utf8_lossy(&output.stdout);
    
    Ok(ToolResult {
        call_id,
        output: text.to_string(),
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
// Main execution dispatcher
// ===========================================================================

async fn execute_computer_use<'a>(call: &ToolCall<'a>) -> anyhow::Result<ToolResult> {
    let action = call.required_str("action")?;
    let call_id = call.call_id.clone();
    let backend = get_platform_backend();

    let action_lower = action.trim().to_lowercase();

    let result = match action_lower.as_str() {
        "capture" => {
            let args = call.args.clone();
            match backend {
                "macos" => capture_macos(&args),
                "linux" => capture_linux(&args),
                "windows" => capture_windows(&args),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "click" | "double_click" | "right_click" | "middle_click" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_pointer_macos(&args, &action_lower, call_id.clone()),
                "linux" => handle_pointer_linux(&args, &action_lower, call_id.clone()),
                "windows" => handle_pointer_windows(&args, &action_lower, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "drag" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_drag_macos(&args, call_id.clone()),
                "linux" => handle_drag_linux(&args, call_id.clone()),
                "windows" => handle_drag_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "scroll" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_scroll_macos(&args, call_id.clone()),
                "linux" => handle_scroll_linux(&args, call_id.clone()),
                "windows" => handle_scroll_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "type" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_type_macos(&args, call_id.clone()),
                "linux" => handle_type_linux(&args, call_id.clone()),
                "windows" => handle_type_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "key" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_key_macos(&args, call_id.clone()),
                "linux" => handle_key_linux(&args, call_id.clone()),
                "windows" => handle_key_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "set_value" => {
            // set_value is macOS-only in the original implementation
            // For other platforms, we'll simulate via type
            let args = call.args.clone();
            match backend {
                "macos" => handle_set_value_macos(&args, call_id.clone()),
                "linux" => handle_set_value_linux(&args, call_id.clone()),
                "windows" => handle_set_value_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "wait" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_wait_macos(&args, call_id.clone()).await,
                "linux" => handle_wait_linux(&args, call_id.clone()),
                "windows" => handle_wait_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        "list_apps" => match backend {
            "macos" => handle_list_apps_macos(call_id.clone()),
            "linux" => handle_list_apps_linux(call_id.clone()),
            "windows" => handle_list_apps_windows(call_id.clone()),
            _ => Err(anyhow::anyhow!("Unsupported platform")),
        },
        "focus_app" => {
            let args = call.args.clone();
            match backend {
                "macos" => handle_focus_app_macos(&args, call_id.clone()),
                "linux" => handle_focus_app_linux(&args, call_id.clone()),
                "windows" => handle_focus_app_windows(&args, call_id.clone()),
                _ => Err(anyhow::anyhow!("Unsupported platform")),
            }
        }
        _ => Err(anyhow::anyhow!("Unknown action: '{action_lower}'")),
    };
    
    let tr = match result {
        Ok(tr) => tr,
        Err(e) => ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some(e.to_string()),
        },
    };
    Ok(tr)
}

// Platform-specific set_value handlers
fn handle_set_value_macos(args: &Value, call_id: String) -> Result<ToolResult> {
    let val = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("set_value requires 'value' argument"))?;
    let element = args.get("element").and_then(|v| v.as_i64());
    let app = args.get("app").and_then(|v| v.as_str());

    let (pid, window_id) = resolve_target_windows_macos(app)
        .ok_or_else(|| anyhow::anyhow!("No target window found"))?;

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

fn handle_set_value_linux(args: &Value, call_id: String) -> Result<ToolResult> {
    let val = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("set_value requires 'value' argument"))?;

    // On Linux, we can use xdotool to type the value
    call_xdo_type(val)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Linux set_value: typed '{val}'"),
        error: None,
    })
}

fn handle_set_value_windows(args: &Value, call_id: String) -> Result<ToolResult> {
    let val = args
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("set_value requires 'value' argument"))?;

    call_pyautogui_type(val)?;
    
    Ok(ToolResult {
        call_id,
        output: format!("Windows set_value: typed '{val}'"),
        error: None,
    })
}

// ===========================================================================
// Tool Definition
// ===========================================================================

fn make_computer_use_tool() -> ToolMeta {
    let backend = get_platform_backend();
    let description = match backend {
        "macos" => "Drive the desktop in the background — screenshots, mouse, keyboard, scroll, drag — without stealing the user's cursor, keyboard focus, or Space. macOS only; requires cua-driver to be installed.",
        "linux" => "Drive the Linux desktop in the background — screenshots, mouse, keyboard, scroll, drag — using xdotool/xdo. Requires xautomation package.",
        "windows" => "Drive the Windows desktop in the background — screenshots, mouse, keyboard, scroll, drag — using pyautogui. Requires Python with pyautogui module.",
        _ => "Drive the desktop in the background — screenshots, mouse, keyboard, scroll, drag — using platform-specific tools.",
    };
    
    ToolMeta {
        name: "computer_use".into(),
        description: description.into(),
        parameters: ToolParameters::JsonSchema {
            schema: computer_use_schema(),
        },
    }
}

// ===========================================================================
// Registration
// ===========================================================================

pub struct ComputerUseTool;

#[async_trait::async_trait]
impl Tool for ComputerUseTool {
    fn name(&self) -> &str {
        "computer_use"
    }
    fn description(&self) -> &str {
        let backend = get_platform_backend();
        match backend {
            "macos" => "Drive the macOS desktop in the background",
            "linux" => "Drive the Linux desktop in the background",
            "windows" => "Drive the Windows desktop in the background",
            _ => "Drive the desktop in the background",
        }
    }
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        execute_computer_use(call).await.unwrap_or_else(|e| ToolResult {
            call_id: call.call_id.clone(),
            output: String::new(),
            error: Some(e.to_string()),
        })
    }
    fn clone_tool(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut ToolRegistry) {
    let tool = Box::new(ComputerUseTool);
    registry.register_with_def(tool, make_computer_use_tool());
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
        crate::computer_use::register(&mut r);
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

    /// Given: All 14 actions in schema enum
    /// When: Enum is checked
    /// Then: Correct count
    #[test]
    fn test_schema_actions_count() {
        let schema = computer_use_schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(actions.len(), 14);
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
