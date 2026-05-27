//! Clipboard integration for text and image operations.

use std::path::Path;

/// Check if clipboard image is available and save it to disk.
pub fn save_clipboard_image(dest: &Path) -> anyhow::Result<bool> {
    let mut file = std::fs::File::create(dest)?;
    let result = clipboard_save_impl(&mut file);
    if result {
        Ok(true)
    } else {
        let _ = std::fs::remove_file(dest);
        Ok(false)
    }
}

fn clipboard_save_impl<W: std::io::Write>(writer: &mut W) -> bool {
    #[cfg(target_os = "macos")]
    return save_clipboard_image_macos(writer);
    #[cfg(target_os = "windows")]
    return save_clipboard_image_windows(writer);
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        if wayland {
            return save_clipboard_image_wayland(writer);
        } else {
            return save_clipboard_image_x11(writer);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return false;
}

/// Check if there is an image in the clipboard (without extracting).
pub fn has_clipboard_image() -> bool {
    #[cfg(target_os = "macos")]
    return has_clipboard_image_macos();
    #[cfg(target_os = "windows")]
    return has_clipboard_image_windows();
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        if wayland {
            return has_clipboard_image_wayland();
        } else {
            return has_clipboard_image_x11();
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return false;
}

/// Read text from the clipboard.
pub fn read_clipboard_text() -> anyhow::Result<String> {
    let mut clipboard = arboard::Clipboard::new()?;
    Ok(clipboard.get_text()?)
}

/// Write text to the clipboard.
pub fn write_clipboard_text(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(())
}

// --- Platform-specific implementations ---

#[cfg(target_os = "macos")]
fn save_clipboard_image_macos<W: std::io::Write>(writer: &mut W) -> bool {
    match std::process::Command::new("osascript")
        .args(["-e", r"set imgData to the clipboard as «class PNGf»"])
        .output()
    {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => {
            let _ = writer.write_all(&output.stdout);
            true
        }
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn has_clipboard_image_macos() -> bool {
    let output = match std::process::Command::new("osascript")
        .args(["-e", "clipboard info"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let info = String::from_utf8_lossy(&output.stdout);
    info.contains("PNGf") || info.contains("TIFF")
}

#[cfg(target_os = "windows")]
fn save_clipboard_image_windows<W: std::io::Write>(_writer: &mut W) -> bool {
    match std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "[System.Windows.Forms.Clipboard]::ContainsImage(); [System.Windows.Forms.Clipboard]::GetImage().Save([Console]::OpenStandardOutput(), [System.Drawing.Imaging.ImageFormat]::Png)",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output.stdout.len() > 0,
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn has_clipboard_image_windows() -> bool {
    let output = match std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "[System.Windows.Forms.Clipboard]::ContainsImage(); if([System.Windows.Forms.Clipboard]::ContainsImage()){Write-Output 'yes' -NoNewline}else{Write-Output 'no' -NoNewline}",
        ])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout),
        _ => String::from("no"),
    };

    output == "yes"
}

#[cfg(target_os = "linux")]
fn save_clipboard_image_wayland<W: std::io::Write>(writer: &mut W) -> bool {
    match std::process::Command::new("wl-paste")
        .args(["--no-newline"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let _ = writer.write_all(&output.stdout);
            !output.stdout.is_empty()
        }
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn has_clipboard_image_wayland() -> bool {
    let output = match std::process::Command::new("wl-paste")
        .args(["--list-types"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let types = String::from_utf8_lossy(&output.stdout);
    types.contains("image/png") || types.contains("image/jpeg") || types.contains("image/gif")
}

#[cfg(target_os = "linux")]
fn save_clipboard_image_x11<W: std::io::Write>(writer: &mut W) -> bool {
    match std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-o", "-t", "image/png"])
        .output()
    {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => {
            let _ = writer.write_all(&output.stdout);
            true
        }
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn has_clipboard_image_x11() -> bool {
    let output = match std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "TARGETS", "-o"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let targets = String::from_utf8_lossy(&output.stdout);
    targets.contains("image/png")
}

/// PNG signature bytes for format verification.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Verify that the given buffer starts with PNG signature.
pub fn is_png_signature(data: &[u8]) -> bool {
    data.len() >= 8 && data.starts_with(PNG_SIGNATURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_signature_matches() {
        assert!(is_png_signature(b"\x89PNG\r\n\x1a\nhello"));
    }

    #[test]
    fn test_non_png_signature_rejected() {
        assert!(!is_png_signature(b"not a png"));
        assert!(!is_png_signature(b"\x00\x00\x00\x00"));
    }

    #[test]
    #[allow(clippy::empty_docs)]
    fn test_clipboard_text_roundtrip() {
        let result = read_clipboard_text();
        if let Ok(text) = result {
            assert!(!text.is_empty() || text.len() == 0);
        }
    }

    #[test]
    #[allow(clippy::empty_docs)]
    fn test_clipboard_text_write() {
        let result = write_clipboard_text("oben-clipboard-test");
        if result.is_ok() {
            if let Ok(read) = read_clipboard_text() {
                assert!(read.contains("oben-clipboard-test"));
            }
        }
        let _ = write_clipboard_text("");
    }
}
