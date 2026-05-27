//! File safety — read/write restrictions and device path blocking.

use std::fs;
use std::io;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;

/// Maximum characters allowed per file read operation (default 100,000).
pub const MAX_READ_CHARS: usize = 100_000;

/// Size threshold where a large-file warning is shown (512 KB in chars).
pub const LARGE_FILE_THRESHOLD: usize = 512_000;

static WRITE_DENIED_EXACT: Lazy<Vec<&str>> = Lazy::new(|| {
    vec![
        "~/.ssh/authorized_keys",
        "~/.ssh/id_rsa",
        "~/.ssh/id_ed25519",
        "~/.ssh/config",
        "~/.hermes/.env",
        "~/.bashrc",
        "~/.zshrc",
        "~/.profile",
        "~/.bash_profile",
        "~/.zprofile",
        "~/.netrc",
        "~/.pgpass",
        "~/.npmrc",
        "~/.pypirc",
        "/etc/sudoers",
        "/etc/passwd",
        "/etc/shadow",
    ]
});

static WRITE_DENIED_PREFIXES: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "~/.ssh/",
        "~/.aws/",
        "~/.gnupg/",
        "~/.kube/",
        "/etc/sudoers.d/",
        "/etc/systemd",
        "~/.docker/",
        "~/.azure/",
        "~/.config/gh/",
    ]
});

static BLOCKED_DEVICE_PATHS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    vec![
        "/dev/zero",
        "/dev/random",
        "/dev/urandom",
        "/dev/full",
        "/dev/stdin",
        "/dev/tty",
        "/dev/console",
        "/dev/stdout",
        "/dev/stderr",
        "/dev/fd/0",
        "/dev/fd/1",
        "/dev/fd/2",
    ]
});

static SENSITIVE_PATH_PREFIXES: &[&str] =
    &["/etc/", "/boot/", "/usr/lib/systemd/", "/private/etc/", "/private/var/"];

static SENSITIVE_EXACT_PATHS: Lazy<Vec<&str>> = Lazy::new(|| {
    vec!["/var/run/docker.sock", "/run/docker.sock"]
});

fn expand_home(path_str: &str) -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        if let Some(stripped) = path_str.strip_prefix("~/") {
            return home.join(stripped);
        }
        if path_str == "~" {
            return home;
        }
    }
    PathBuf::from(path_str)
}

fn resolve_path(path: &Path) -> PathBuf {
    let expanded = expand_home(path.to_string_lossy().as_ref());
    if let Ok(c) = expanded.canonicalize() {
        c
    } else {
        expanded
    }
}

/// Check if a path is in the write deny list.
pub fn is_write_denied(path: &Path) -> Option<String> {
    let resolved = resolve_path(path);
    let resolved_str = resolved.to_string_lossy();

    for denied in WRITE_DENIED_EXACT.iter() {
        let denied_resolved = resolve_path(Path::new(denied));
        let denied_str = denied_resolved.to_string_lossy();
        if resolved_str == denied_str {
            return Some(format!(
                "Write denied: {} is explicitly blocked.",
                resolved.display()
            ));
        }
    }

    for prefix in WRITE_DENIED_PREFIXES.iter() {
        let prefix_expanded = expand_home(prefix);
        let prefix_str = prefix_expanded.to_string_lossy();
        if resolved_str.starts_with(prefix_str.as_ref()) {
            return Some(format!(
                "Write denied: {} is in a restricted directory.",
                resolved_str
            ));
        }
    }

    if let Ok(safe_root) = std::env::var("HERMES_WRITE_SAFE_ROOT") {
        if let Ok(root) = PathBuf::from(&safe_root).canonicalize() {
            if !resolved.starts_with(&root) {
                return Some(format!(
                    "Write denied: {} is outside safe root {}",
                    resolved.display(),
                    root.display()
                ));
            }
        }
    }

    None
}

pub fn is_blocked_device(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if BLOCKED_DEVICE_PATHS.contains(&s.as_ref()) {
        return true;
    }
    if s.starts_with("/proc/") && (s.ends_with("/fd/0") || s.ends_with("/fd/1") || s.ends_with("/fd/2"))
    {
        return true;
    }
    false
}

pub fn check_sensitive_path(path: &Path) -> Option<String> {
    let s = path.to_string_lossy();
    for prefix in SENSITIVE_PATH_PREFIXES {
        if s.starts_with(*prefix) {
            return Some(format!(
                "Sensitive path: {} is under {}. For privileged writes, use the terminal tool with sudo.",
                s, prefix
            ));
        }
    }
    if SENSITIVE_EXACT_PATHS.contains(&s.as_ref()) {
        return Some(format!(
            "Sensitive path: {} is a privileged resource. Use the terminal tool with sudo.",
            s
        ));
    }
    None
}

pub fn file_read_max_chars() -> usize {
    std::env::var("HERMES_FILE_READ_MAX_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_READ_CHARS)
}

/// Read a file with safety checks and character limits.
pub fn safe_read_file(path: &Path) -> io::Result<String> {
    if is_blocked_device(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Blocked device path: {}", path.display()),
        ));
    }

    if let Some(err) = is_write_denied(path) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, err));
    }

    let contents = fs::read_to_string(path)?;

    let max_chars = file_read_max_chars();
    if contents.len() > LARGE_FILE_THRESHOLD {
        tracing::warn!(
            "File {} is large ({} chars, limit: {}). Consider using offset+limit.",
            path.display(),
            contents.len(),
            max_chars
        );
    }

    if contents.len() > max_chars {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "File {} is too large ({} chars, limit: {}). Consider using offset+limit reads.",
                path.display(),
                contents.len(),
                max_chars
            ),
        ));
    }

    Ok(contents)
}

/// Read the tail of a log-style file, respecting a max byte limit.
pub fn read_tail(path: &Path, max_bytes: usize, line_tail: usize) -> io::Result<String> {
    let metadata = match path.metadata() {
        Ok(m) => m,
        Err(_) => return Ok(String::new()),
    };

    let file_size = metadata.len() as usize;
    if file_size == 0 {
        return Ok(String::new());
    }

    let bytes_to_read = max_bytes.min(file_size);
    let start_offset = file_size.saturating_sub(bytes_to_read);

    let mut file = fs::File::open(path)?;
    let mut buf = Vec::with_capacity(bytes_to_read);
    file.seek(SeekFrom::Start(start_offset as u64))?;
    file.read_to_end(&mut buf)?;

    let text = String::from_utf8_lossy(&buf).into_owned();

    let lines: Vec<&str> = text.lines().collect();
    if line_tail > 0 && lines.len() > line_tail {
        return Ok(lines[lines.len() - line_tail..].join("\n"));
    }

    Ok(text)
}

/// Normalize a bundle path for skill installation, blocking traversal.
pub fn normalize_bundle_path(path: &str, allow_nested: bool) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err("Absolute paths are not allowed".to_string());
    }
    #[cfg(windows)]
    if trimmed.len() >= 2 && trimmed.chars().nth(1) == Some(':') {
        return Err("Absolute paths are not allowed".to_string());
    }

    let normalized = trimmed.replace('\\', "/");

    for component in normalized.split('/') {
        if component == ".." {
            return Err("Path traversal detected: '..' components are not allowed".to_string());
        }
        if component.is_empty() && normalized != "/" {
            return Err("Invalid path: null components".to_string());
        }
    }

    if !allow_nested && normalized.contains('/') {
        return Err("Nested paths are not allowed for this field".to_string());
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_authorized_keys_denied() {
        let home = dirs::home_dir().expect("home dir");
        let result = is_write_denied(&home.join(".ssh/authorized_keys"));
        assert!(result.is_some());
    }

    #[test]
    fn test_safe_file_is_allowed() {
        let home = dirs::home_dir().expect("home dir");
        let result = is_write_denied(&home.join("Documents/readme.txt"));
        assert!(result.is_none());
    }

    #[test]
    fn test_blocked_devices() {
        assert!(is_blocked_device(Path::new("/dev/zero")));
        assert!(is_blocked_device(Path::new("/dev/stdin")));
        assert!(!is_blocked_device(Path::new("/dev/null")));
    }

    #[test]
    fn test_sensitive_path() {
        let result = check_sensitive_path(Path::new("/etc/shadow"));
        assert!(result.is_some());
    }

    #[test]
    fn test_normal_path_not_sensitive() {
        let result = check_sensitive_path(Path::new("/home/user/project/main.rs"));
        assert!(result.is_none());
    }

    #[test]
    fn test_path_traversal_rejected() {
        let result = normalize_bundle_path("../etc/passwd", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_absolute_path_rejected() {
        let result = normalize_bundle_path("/absolute/path", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_path_rejected() {
        let result = normalize_bundle_path("", true);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_path_accepted() {
        let result = normalize_bundle_path("category/skill-name", true);
        assert_eq!(result.unwrap(), "category/skill-name");
    }

    #[test]
    fn test_single_component_allowed_without_nested() {
        let result = normalize_bundle_path("my-skill", false);
        assert_eq!(result.unwrap(), "my-skill");
    }

    #[test]
    fn test_nested_path_rejected_without_nested() {
        let result = normalize_bundle_path("category/skill", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_backslash_normalization() {
        let result = normalize_bundle_path("category\\skill-name", true);
        assert_eq!(result.unwrap(), "category/skill-name");
    }

    #[test]
    fn test_docker_socket_sensitive() {
        let result = check_sensitive_path(Path::new("/var/run/docker.sock"));
        assert!(result.is_some());
    }
}
