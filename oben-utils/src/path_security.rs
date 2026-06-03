/// Path security helpers — prevent directory traversal attacks.
use std::path::{Path, PathBuf};

/// Resolve a user-supplied path against a base directory, preventing traversal.
pub fn sanitize_path(base: &Path, user_path: &str) -> anyhow::Result<PathBuf> {
    let resolved = base.join(user_path);
    let canonicalized = resolved.canonicalize()?;
    if !canonicalized.starts_with(base) {
        anyhow::bail!(
            "Path traversal detected: {} is outside {}",
            canonicalized.display(),
            base.display()
        );
    }
    Ok(canonicalized)
}

/// Check if a path is safe for shell command execution.
pub fn is_path_safe(path: &Path) -> bool {
    let s = path.to_string_lossy();
    // Reject paths with shell metacharacters or newlines
    !s.contains(|c: char| matches!(c, ';' | '&' | '|' | '$' | '`' | '(' | ')' | '\n' | '\r'))
}
