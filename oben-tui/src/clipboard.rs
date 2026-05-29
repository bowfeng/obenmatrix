//! Cross-platform native clipboard access using OS-native CLI tools.
//!
//! Tools used (no Rust dependencies):
//! - macOS: `pbpaste` / `pbcopy`
//! - Linux + Wayland: `wl-paste` / `wl-copy`
//! - Linux + WSL: `powershell.exe`
//! - Linux + X11 / others: `xclip`

use std::io::Write;
use std::process::{Command, Stdio};

const MAX_READ_BUFFER: u32 = 4 * 1024 * 1024; // 4MB

/// Read clipboard content as a usable UTF-8 string.
///
/// Tries platform-appropriate tools in order and returns `None` if
/// all backends fail or the content is not valid clipboard text.
pub fn read_clipboard() -> Option<String> {
    let stdout = read_clipboard_raw()?;
    let text = String::from_utf8(stdout).ok()?;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(String::new());
    }

    if !is_usable_clipboard_text(&trimmed) {
        return None;
    }

    Some(trimmed.to_string())
}

/// Write text to the system clipboard via native tools.
///
/// Returns `true` if at least one backend accepted the data.
pub fn write_clipboard(text: &str) -> bool {
    if text.is_empty() {
        return write_clipboard_raw("").is_ok();
    }

    write_clipboard_raw(text).is_ok()
}

// ── internal helpers ────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn read_clipboard_raw() -> Option<Vec<u8>> {
    let output = Command::new("pbpaste")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    Some(output.stdout)
}

#[cfg(target_os = "macos")]
fn write_clipboard_raw(text: &str) -> std::io::Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let _ = child.wait();
    Ok(())
}

#[cfg(target_os = "linux")]
fn read_clipboard_raw() -> Option<Vec<u8>> {
    let env = std::env::vars().collect::<std::collections::HashMap<_, _>>();

    // WSL via powershell.exe
    if env.get("WSL_INTEROP").is_some() || env.get("WSL_DISTRO_NAME").is_some() {
        if let Ok(output) = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Clipboard -Raw",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            return Some(output.stdout);
        }
    }

    // Wayland via wl-paste
    if env.get("WAYLAND_DISPLAY").is_some() {
        if let Ok(output) = Command::new("wl-paste")
            .args(["--type", "text"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            return Some(output.stdout);
        }
    }

    // X11 / fallback via xclip
    if let Ok(output) = Command::new("xclip")
        .args(["-selection", "clipboard", "-out"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        return Some(output.stdout);
    }

    None
}

#[cfg(target_os = "linux")]
fn write_clipboard_raw(text: &str) -> std::io::Result<()> {
    let env = std::env::vars().collect::<std::collections::HashMap<_, _>>();

    // WSL via powershell.exe
    if env.get("WSL_INTEROP").is_some() || env.get("WSL_DISTRO_NAME").is_some() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Set-Clipboard -Value `$input",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let _ = child.wait();
        return Ok(());
    }

    // Wayland via wl-copy
    if env.get("WAYLAND_DISPLAY").is_some() {
        if let Ok(mut child) = Command::new("wl-copy")
            .args(["--type", "text/plain"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return Ok(());
        }
    }

    // X11 / fallback via xclip
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-in"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let _ = child.wait();
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_clipboard_raw() -> Option<Vec<u8>> {
    // Placeholder for Windows / other platforms; not implemented in this
    // version — will return None so callers get the same "no clipboard"
    // behaviour as when tools are missing.
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn write_clipboard_raw(_text: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "clipboard not supported on this platform",
    ))
}

/// Check whether text is safe to treat as usable clipboard content.
///
/// Rejects content with embedded null bytes or content where more than
/// 50% of the character count consists of suspicious control characters
/// (excluding the normal whitespace chars `\n`, `\r`, `\t`).
pub fn is_usable_clipboard_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    if text.as_bytes().contains(&0) {
        return false;
    }

    let len = text.len();

    // Single-char strings are always considered valid.
    if len == 1 {
        return true;
    }

    let mut suspicious = 0usize;

    for ch in text.chars() {
        let code = ch as u32;
        let is_control = code < 0x20 && ch != '\n' && ch != '\r' && ch != '\t';

        if is_control || ch == '\u{FFFD}' {
            suspicious += 1;
        }
    }

    let threshold = len / 2; // 50 %
    suspicious <= threshold
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_usable_clipboard_valid() {
        assert!(is_usable_clipboard_text("Hello world"));
        assert!(is_usable_clipboard_text("line1\nline2"));
        assert!(is_usable_clipboard_text("a\tb\rc"));
        assert!(is_usable_clipboard_text("x"));
    }

    #[test]
    fn test_is_usable_clipboard_null_byte() {
        assert!(!is_usable_clipboard_text("hello\x00world"));
        assert!(!is_usable_clipboard_text("\x00"));
    }

    #[test]
    fn test_is_usable_clipboard_empty() {
        // is_usable_clipboard_text returns false for empty string by
        // design — empty content is handled by the caller as a valid
        // empty clipboard rather than invalid content.
        assert!(!is_usable_clipboard_text(""));
    }

    #[test]
    fn test_is_usable_clipboard_too_many_control_chars() {
        let mut text = String::new();
        for _ in 0..10 {
            text.push('\x01');
        }
        text.push_str("Hello world");
        // 10 suspicious out of 21 chars  > 50 % (10 > 10 is false, so this
        // is right at the boundary — let's push it over).
        let mut more = text.clone();
        for _ in 0..5 {
            more.push('\x02');
        }
        // 15 suspicious out of 26 chars  > 50 % (13)
        // so this should be rejected.
        assert!(!is_usable_clipboard_text(&more));
    }

    #[test]
    fn test_read_clipboard_fails_gracefully() {
        // On headless systems or when no clipboard content exists we
        // expect `None`; on macOS with real clipboard content we get
        // the actual text.  Either way must not panic.
        let result = read_clipboard();
        let _ = result;
    }

    #[test]
    fn test_write_clipboard_fails_gracefully() {
        // Writing to a nonexistent / invalid clipboard tool should return
        // `false` (not panic).
        let result = write_clipboard("test");
        // If we are lucky enough to have `pbcopy` / `xclip` available,
        // the test passes either way.  On headless / CI we expect `false`.
        let _ = result;
    }
}
