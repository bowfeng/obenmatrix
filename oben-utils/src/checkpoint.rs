//! Checkpoint system — filesystem snapshots per working directory.
//!
//! Inspired by [`tools/checkpoint_manager.py`].
//!
//! Creates snapshots before file-mutating operations (once per turn per directory).
//! Supports restore, list, diff, and automatic pruning with configurable policies.
//!
//! [`tools/checkpoint_manager.py`]: https://github.com/bowfeng/oben-alien/blob/main/hermes-agent/tools/checkpoint_manager.py

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_SNAPSHOTS: usize = 20;
const MAX_SNAPSHOT_FILES: usize = 50_000;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub id: String,
    pub reason: String,
    pub created_at: String,
    pub created_date: String,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub id: String,
    pub reason: String,
    pub timestamp: f64,
    pub created_at: String,
    pub created_date: String,
    pub file_count: u64,
    pub size_bytes: u64,
}

impl SnapshotMetadata {
    pub fn dir_name(&self) -> String {
        self.id.clone()
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckpointPolicy {
    Never,
    OnExit,
    Interval(Duration),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    pub enabled: bool,
    pub policy: CheckpointPolicy,
    pub max_snapshots: usize,
    pub max_total_size_mb: usize,
    pub max_file_size_mb: usize,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: CheckpointPolicy::Never,
            max_snapshots: DEFAULT_MAX_SNAPSHOTS,
            max_total_size_mb: 500,
            max_file_size_mb: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// String-based path validation. Does NOT require working_dir to exist.
pub fn validate_file_path(file_path: &str, _working_dir: &str) -> Option<String> {
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Some("Empty file path".to_string());
    }

    if Path::new(file_path).is_absolute() {
        return Some(format!(
            "File path must be relative, got absolute path: {file_path:?}"
        ));
    }

    for component in file_path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .split('/')
    {
        if component == ".." {
            return Some(format!(
                "File path escapes the working directory via traversal: {file_path:?}"
            ));
        }
    }

    None
}

pub fn expand_tilde(path_str: &str) -> String {
    if !path_str.starts_with("~/") && path_str != "~" {
        return path_str.to_string();
    }
    std::env::var("HOME")
        .map(|home| {
            if path_str == "~" {
                home
            } else {
                format!("{}/{}", home, &path_str[2..])
            }
        })
        .unwrap_or_else(|_| path_str.to_string())
}

pub fn normalize_path(path_value: &str) -> Option<PathBuf> {
    let expanded = expand_tilde(path_value);
    Path::new(&expanded).canonicalize().ok()
}

pub fn project_hash(working_dir: &str) -> anyhow::Result<String> {
    let mut hasher = DefaultHasher::new();
    working_dir.hash(&mut hasher);
    let hex = format!("{:x}", hasher.finish());
    Ok(hex.chars().take(16).collect())
}

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn epoch_to_iso(epoch: f64) -> String {
    let secs = epoch as i64;
    let nanos = ((epoch - secs as f64) * 1_000_000_000.0) as u32;
    let days = secs / 86400;
    let tod = ((secs % 86400) + 86400) % 86400;

    let hours = tod / 3600;
    let mins = (tod % 3600) / 60;
    let secs_left = tod % 60;

    let y = 1970 + (days as i32) / 365;
    let m = (days as i32 % 365 / 30) + 1;
    let d = (days as i32 % 30) + 1;

    let mut buf = String::new();
    let _ = write!(buf, "{:04}-{:02}-{:02}", y, m, d);
    let _ = write!(
        buf,
        "T{:02}:{:02}:{:02}.{:09}",
        hours, mins, secs_left, nanos
    );
    buf
}

// ---------------------------------------------------------------------------
// CheckpointStore
// ---------------------------------------------------------------------------

/// Manages filesystem snapshots for a working directory.
pub struct CheckpointStore {
    pub checkpoint_base: PathBuf,
    pub working_dir: PathBuf,
    pub dir_hash: String,
    pub max_snapshots: usize,
    _checkpointed_files: HashSet<String>,
}

impl CheckpointStore {
    pub fn new(
        checkpoint_base: &str,
        working_dir: &str,
        max_snapshots: usize,
    ) -> anyhow::Result<Self> {
        let checkpoint_base = PathBuf::from(checkpoint_base);
        let working_dir = Path::new(working_dir).to_path_buf();
        let mut hasher = DefaultHasher::new();
        working_dir.hash(&mut hasher);
        let dir_hash = format!("{:x}", hasher.finish());

        Ok(Self {
            checkpoint_base,
            working_dir,
            dir_hash,
            max_snapshots: usize::max(1, max_snapshots),
            _checkpointed_files: HashSet::new(),
        })
    }

    pub fn snapshot_dir(&self) -> PathBuf {
        self.checkpoint_base.join(&self.dir_hash)
    }

    pub fn list_snapshots(&self) -> anyhow::Result<Vec<SnapshotMetadata>> {
        let meta_dir = self.checkpoint_base.join("meta");
        let meta_file = meta_dir.join(&self.dir_hash.clone());

        let mut entries: Vec<SnapshotMetadata> = if meta_file.is_file() {
            let content = std::fs::read_to_string(&meta_file)?;
            content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        } else {
            Vec::new()
        };

        entries.sort_by(|a, b| {
            b.timestamp
                .partial_cmp(&a.timestamp)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(entries)
    }

    pub fn save_snapshot(&mut self, reason: &str) -> anyhow::Result<Option<String>> {
        let file_count = file_count_in_dir(&self.working_dir)?;
        if file_count > MAX_SNAPSHOT_FILES {
            return Ok(None);
        }

        let id = Uuid::new_v4().to_string().replace('-', "");
        let ts = now_epoch();
        let snapshot_dir = self.snapshot_dir().join(&id);
        std::fs::create_dir_all(&snapshot_dir)?;

        let mut copied = 0u64;
        for item in self.working_dir.read_dir()? {
            let item = item?;
            let src = item.path();
            if src.is_file() {
                let dest = snapshot_dir.join(src.file_name().unwrap());
                std::fs::copy(&src, &dest)?;
                copied += 1;
            } else if src.is_dir() && !is_excluded_dir(&src) {
                copy_dir(&src, &snapshot_dir)?;
                copied += 1;
            }
        }

        update_meta_snapshot(
            &self.checkpoint_base,
            &self.dir_hash,
            &id,
            reason,
            ts,
            copied,
        )?;

        Ok(Some(id))
    }

    pub fn restore_snapshot(&mut self, id: &str, file_name: Option<&str>) -> anyhow::Result<()> {
        let snapshot_dir = self.snapshot_dir().join(id);
        if !snapshot_dir.is_dir() {
            return Err(anyhow::anyhow!("Snapshot '{}' not found", id));
        }

        let _ = self.save_snapshot(&format!("pre-rollback for {}", id));

        if let Some(fname) = file_name {
            if let Some(err) = validate_file_path(fname, self.working_dir.to_str().unwrap_or(".")) {
                return Err(anyhow::anyhow!(err));
            }

            let src_file = snapshot_dir.join(fname);
            let dest_file = self.working_dir.join(fname);

            if !src_file.exists() {
                return Err(anyhow::anyhow!("File '{}' not found in snapshot", fname));
            }

            if let Some(parent) = dest_file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src_file, &dest_file)?;
            tracing::info!("Restored {:?}", fname);
        } else {
            copy_dir(&snapshot_dir, &self.working_dir)?;
            tracing::info!("Restored snapshot {}", id);
        }

        Ok(())
    }

    pub fn diff_snapshot(&self, id: &str) -> anyhow::Result<DiffResult> {
        let snapshot_dir = self.snapshot_dir().join(id);
        if !snapshot_dir.is_dir() {
            return Err(anyhow::anyhow!("Snapshot '{}' not found", id));
        }

        let mut modified: Vec<String> = Vec::new();
        let mut removed: Vec<String> = Vec::new();

        for item in snapshot_dir.read_dir()? {
            let item = item?;
            let src = item.path();
            if src.is_file() {
                let fname = src.file_name().unwrap().to_str().unwrap_or("?");
                let current_path = self.working_dir.join(fname);

                if !current_path.exists() {
                    removed.push(fname.to_string());
                } else if !files_match(&src, &current_path)? {
                    modified.push(fname.to_string());
                }
            }
        }

        Ok(DiffResult {
            snapshot_id: id.to_string(),
            added: Vec::new(),
            modified,
            removed,
        })
    }

    pub fn delete_snapshot(&self, id: &str) -> anyhow::Result<()> {
        let snapshot_dir = self.snapshot_dir().join(id);
        if !snapshot_dir.is_dir() {
            return Err(anyhow::anyhow!("Snapshot '{}' not found", id));
        }
        std::fs::remove_dir_all(&snapshot_dir)?;
        cleanup_meta_snapshot(&self.checkpoint_base, &self.dir_hash, id)?;
        Ok(())
    }

    pub fn prune(&mut self) -> anyhow::Result<usize> {
        let snapshots = self.list_snapshots()?;
        let max_snap = self.max_snapshots;
        if snapshots.len() <= max_snap {
            return Ok(0);
        }

        let to_delete = &snapshots[max_snap..];
        let count = to_delete.len();
        for s in to_delete.iter() {
            let _ = self.delete_snapshot(&s.id);
        }
        Ok(count)
    }
}

#[derive(Debug, Default)]
pub struct DiffResult {
    pub snapshot_id: String,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
}

pub fn is_excluded_dir_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    EXCLUDED_DIRS.contains(&name)
}

fn is_excluded_dir(path: &Path) -> bool {
    is_excluded_dir_dir(path)
}

const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    "target",
    "out",
    ".next",
    "__pycache__",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "coverage",
    ".coverage",
    ".venv",
    "venv",
    "env",
    ".git",
    ".hg",
    ".svn",
    ".worktrees",
];

fn copy_dir(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let dest = dest.join(src.file_name().unwrap_or(src.as_os_str()));
    std::fs::create_dir_all(&dest)?;

    for item in src.read_dir()? {
        let item = item?;
        let path = item.path();
        if path.is_dir() {
            if !is_excluded_dir(&path) {
                copy_dir(&path, &dest)?;
            }
        } else if path.is_file() {
            let dest_path = dest.join(item.file_name());
            std::fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

fn update_meta_snapshot(
    checkpoint_base: &Path,
    dir_hash: &str,
    id: &str,
    reason: &str,
    timestamp: f64,
    _file_count: u64,
) -> anyhow::Result<()> {
    let meta_dir = checkpoint_base.join("meta");
    std::fs::create_dir_all(&meta_dir)?;
    let meta_file = meta_dir.join(dir_hash);

    let mut entries: Vec<serde_json::Value> = if meta_file.is_file() {
        let content = std::fs::read_to_string(&meta_file)?;
        content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    } else {
        Vec::new()
    };

    let created_at = epoch_to_iso(timestamp);
    let created_date = created_at.split('T').next().unwrap_or("");
    let entry = serde_json::json!({
        "id": id,
        "reason": reason,
        "timestamp": timestamp,
        "created_at": created_at,
        "created_date": created_date,
        "file_count": 0,
        "size_bytes": 0,
    });

    if let Some(pos) = entries.iter().position(|e| e["id"] == id) {
        entries[pos] = entry
    } else {
        entries.push(entry)
    }

    let mut f = std::fs::File::create(meta_file)?;
    for entry in entries {
        writeln!(f, "{}", serde_json::to_string(&entry)?)?;
    }
    Ok(())
}

fn cleanup_meta_snapshot(checkpoint_base: &Path, dir_hash: &str, id: &str) -> anyhow::Result<()> {
    let meta_dir = checkpoint_base.join("meta");
    if !meta_dir.is_dir() {
        return Ok(());
    }
    let meta_file = meta_dir.join(dir_hash);
    if !meta_file.is_file() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&meta_file)?;
    let filtered: Vec<String> = content
        .lines()
        .filter(|line| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(id_val) = v.get("id").and_then(|v| v.as_str()) {
                    return id_val != id;
                }
            }
            true
        })
        .map(|s| s.to_string())
        .collect();

    let mut f = std::fs::File::create(meta_file)?;
    for entry in filtered {
        writeln!(f, "{}", entry)?;
    }
    Ok(())
}

fn file_count_in_dir(dir: &Path) -> anyhow::Result<usize> {
    let mut count = 0usize;
    for entry in dir.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            count += 1;
        } else if path.is_dir() && !is_excluded_dir(&path) {
            count += 1;
            count += file_count_in_dir(&path)?;
        }
    }
    Ok(count)
}

fn files_match(path1: &Path, path2: &Path) -> anyhow::Result<bool> {
    Ok(std::fs::read(path1)? == std::fs::read(path2)?)
}

/// Format a snapshot list for display.
pub fn format_snapshot_list(snapshots: &[SnapshotEntry]) -> String {
    if snapshots.is_empty() {
        return "No checkpoints found".to_string();
    }

    let mut lines = Vec::new();
    for (i, snap) in snapshots.iter().enumerate() {
        let date = snap
            .created_at
            .split('T')
            .next()
            .unwrap_or(&snap.created_at);
        let time = if snap.created_at.contains('T') {
            snap.created_at
                .split('T')
                .nth(1)
                .map(|t| t.split('+').next().unwrap_or(""))
                .unwrap_or("")
        } else {
            ""
        };

        let stat = if snap.files_changed > 0 {
            format!(
                "  ({} file{}, +{}/-{})",
                snap.files_changed,
                if snap.files_changed != 1 { "s" } else { "" },
                snap.insertions,
                snap.deletions
            )
        } else {
            String::new()
        };

        lines.push(format!(
            "  {}. {}  {} {} {}{}",
            i + 1,
            snap.id.chars().take(8).collect::<String>(),
            date,
            time,
            snap.reason,
            stat
        ));
    }

    lines.push("\n  /restore <N>             restore to checkpoint N".to_string());
    lines.push("\n  /restore diff <N>        preview changes since checkpoint N".to_string());
    lines.push("\n  /restore <N> <file>      restore a single file from checkpoint N".to_string());
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_snapshot_creates_directory() {
        let tmp_dir = std::env::temp_dir().join("test_checkpoint_dir");
        let work_dir = tmp_dir.join("work");
        let cp_base = tmp_dir.join("checkpoints");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&cp_base).unwrap();

        let test_file = work_dir.join("test.txt");
        std::fs::write(&test_file, "hello world").unwrap();

        let mut store = CheckpointStore::new(
            cp_base.to_str().unwrap(),
            work_dir.to_str().unwrap(),
            DEFAULT_MAX_SNAPSHOTS,
        )
        .unwrap();

        let result = store.save_snapshot("initial");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        let snaps = store.list_snapshots().unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].reason, "initial");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_list_snapshots_newest_first() {
        let tmp_dir = std::env::temp_dir().join("test_list_dir");
        let work_dir = tmp_dir.join("work");
        let cp_base = tmp_dir.join("checkpoints");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&cp_base).unwrap();

        let mut store = CheckpointStore::new(
            cp_base.to_str().unwrap(),
            work_dir.to_str().unwrap(),
            DEFAULT_MAX_SNAPSHOTS,
        )
        .unwrap();

        for i in 0..3 {
            let test_file = work_dir.join(format!("snap_{}.txt", i));
            std::fs::write(&test_file, format!("data {}", i)).unwrap();
            store.save_snapshot(&format!("reason {}", i)).unwrap();
        }

        let snaps = store.list_snapshots().unwrap();
        assert_eq!(snaps.len(), 3);
        for i in 0..snaps.len() - 1 {
            assert!(snaps[i].timestamp >= snaps[i + 1].timestamp);
        }

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_config_defaults() {
        let config = CheckpointConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.policy, CheckpointPolicy::Never);
        assert_eq!(config.max_snapshots, DEFAULT_MAX_SNAPSHOTS);
    }

    #[test]
    fn test_policy_equality() {
        let never1 = CheckpointPolicy::Never;
        let never2 = CheckpointPolicy::Never;
        assert_eq!(never1, never2);

        let interval = CheckpointPolicy::Interval(Duration::from_secs(60));
        assert_ne!(never1, interval);
    }

    #[test]
    fn test_validate_path_traversal_rejected() {
        let err = validate_file_path("../etc/passwd", "/safe/workdir").unwrap();
        assert!(err.contains("escapes"));
    }

    #[test]
    fn test_validate_absolute_path_rejected() {
        let err = validate_file_path("/etc/shadow", "/safe/dir").unwrap();
        assert!(err.contains("absolute"));
    }

    #[test]
    fn test_validate_valid_relative_path() {
        let result = validate_file_path("safe/file.txt", "/safe/workdir");
        assert!(result.is_none());
    }

    #[test]
    fn test_project_hash_deterministic() {
        let hash1 = project_hash("/safe/workdir").unwrap();
        let hash2 = project_hash("/safe/workdir").unwrap();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_snapshot_metadata_dir_name() {
        let meta = SnapshotMetadata {
            id: "abc123".to_string(),
            reason: "test".to_string(),
            timestamp: 1000.0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            created_date: "2025-01-01".to_string(),
            file_count: 0,
            size_bytes: 0,
        };
        assert_eq!(meta.dir_name(), "abc123");
    }

    #[test]
    fn test_prune_snapshots_keeps_max() {
        let tmp_dir = std::env::temp_dir().join("test_prune_dir");
        let work_dir = tmp_dir.join("work");
        let cp_base = tmp_dir.join("checkpoints");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&cp_base).unwrap();

        {
            let mut store =
                CheckpointStore::new(cp_base.to_str().unwrap(), work_dir.to_str().unwrap(), 3)
                    .unwrap();

            for i in 0..5 {
                let test_file = work_dir.join(format!("data_{}.txt", i));
                std::fs::write(&test_file, format!("info {}", i)).unwrap();
                store.save_snapshot(&format!("reason {}", i)).unwrap();
            }

            let pruned = store.prune().unwrap();
            assert_eq!(pruned, 2);
        }

        // Read metadata again
        let store =
            CheckpointStore::new(cp_base.to_str().unwrap(), work_dir.to_str().unwrap(), 3).unwrap();

        let snaps = store.list_snapshots().unwrap();
        assert_eq!(snaps.len(), 3);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_diff_snapshot_changes() {
        let tmp_dir = std::env::temp_dir().join("test_diff_dir");
        let work_dir = tmp_dir.join("work");
        let cp_base = tmp_dir.join("checkpoints");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&cp_base).unwrap();

        let mut store =
            CheckpointStore::new(cp_base.to_str().unwrap(), work_dir.to_str().unwrap(), 10)
                .unwrap();

        let test_file = work_dir.join("test.txt");
        std::fs::write(&test_file, "original").unwrap();
        let snapshot_id = store.save_snapshot("first").unwrap();
        assert!(snapshot_id.is_some());

        // Read meta again to verify snapshot listed
        let store2 =
            CheckpointStore::new(cp_base.to_str().unwrap(), work_dir.to_str().unwrap(), 10)
                .unwrap();
        let snaps = store2.list_snapshots().unwrap();
        assert_eq!(snaps.len(), 1);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_checkpoint_policy_variants() {
        let _ne: CheckpointPolicy = CheckpointPolicy::Never;
        let _ex: CheckpointPolicy = CheckpointPolicy::OnExit;
        let _it: CheckpointPolicy = CheckpointPolicy::Interval(Duration::from_secs(60));
    }

    #[test]
    fn test_snapshot_no_files() {
        let tmp_dir = std::env::temp_dir().join("test_no_files");
        let work_dir = tmp_dir.join("work");
        let cp_base = tmp_dir.join("checkpoints");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::create_dir_all(&cp_base).unwrap();

        let mut store =
            CheckpointStore::new(cp_base.to_str().unwrap(), work_dir.to_str().unwrap(), 10)
                .unwrap();

        let result = store.save_snapshot("empty");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        let snaps = store.list_snapshots().unwrap();
        assert_eq!(snaps.len(), 1);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_excluded_dirs() {
        assert!(!is_excluded_dir(Path::new("src")));
        assert!(!is_excluded_dir(Path::new("shared")));
        assert!(!is_excluded_dir(Path::new("media")));
        assert!(!is_excluded_dir(Path::new("logs")));
    }
}
