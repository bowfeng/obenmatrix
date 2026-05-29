use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_ENTRIES: usize = 1000;
const FILENAME: &str = ".oben_history";

fn history_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("obenalien");
    path.push(FILENAME);
    path
}

pub struct InputHistory {
    path: PathBuf,
    inner: Mutex<InputHistoryInner>,
}

struct InputHistoryInner {
    entries: Vec<String>,
    history_idx: Option<usize>,
    current_draft: String,
}

impl InputHistory {
    pub fn new() -> Self {
        let path = history_path();
        let entries = Self::load_entries(&path);
        Self {
            path,
            inner: Mutex::new(InputHistoryInner {
                entries,
                history_idx: None,
                current_draft: String::new(),
            }),
        }
    }

    pub(crate) fn new_at(path: PathBuf) -> Self {
        let entries = Self::load_entries(&path);
        Self {
            path,
            inner: Mutex::new(InputHistoryInner {
                entries,
                history_idx: None,
                current_draft: String::new(),
            }),
        }
    }

    fn load_entries(path: &PathBuf) -> Vec<String> {
        if !path.exists() {
            return Vec::new();
        }
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let all_lines: Vec<String> = reader
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.is_empty())
            .collect();
        let skip = all_lines.len().saturating_sub(MAX_ENTRIES);
        all_lines.into_iter().skip(skip).collect()
    }

    pub fn append(&mut self, line: &str) {
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.entries.last() == Some(&trimmed) {
                return;
            }
            inner.entries.push(trimmed);
            let count = inner.entries.len();
            if count > MAX_ENTRIES {
                inner.entries.drain(..count - MAX_ENTRIES);
            }
        }
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => f,
            Err(_) => return,
        };
        let sentinel = self.inner.lock().unwrap().entries.last().cloned();
        if let Some(last) = sentinel {
            let _ = writeln!(file, "{}", last);
        }
    }

    pub fn history(&self) -> Vec<String> {
        self.inner.lock().unwrap().entries.clone()
    }

    pub fn history_idx(&self) -> Option<usize> {
        self.inner.lock().unwrap().history_idx
    }

    pub fn set_history_idx(&mut self, idx: Option<usize>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(i) = idx {
            if i >= inner.entries.len() {
                inner.history_idx = None;
            } else {
                inner.history_idx = Some(i);
            }
        } else {
            inner.history_idx = None;
        }
    }

    pub fn current_draft(&self) -> String {
        self.inner.lock().unwrap().current_draft.clone()
    }

    pub fn up(&mut self, current_input: &str) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        match inner.history_idx {
            _ if inner.entries.is_empty() => None,
            None => {
                inner.current_draft = current_input.to_string();
                inner.history_idx = Some(inner.entries.len() - 1);
                inner.entries.last().cloned()
            }
            Some(0) => None,
            Some(i) => {
                inner.history_idx = Some(i - 1);
                inner.entries.get(i - 1).cloned()
            }
        }
    }

    pub fn down(&mut self) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        match inner.history_idx {
            Some(0) => {
                inner.history_idx = None;
                Some(inner.current_draft.clone())
            }
            Some(i) if i + 1 == inner.entries.len() => {
                inner.history_idx = None;
                Some(inner.current_draft.clone())
            }
            Some(i) => {
                inner.history_idx = Some(i + 1);
                inner.entries.get(i + 1).cloned()
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn new_test_history(contents: &[&str]) -> (TempDir, InputHistory) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILENAME);
        if !contents.is_empty() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut file = fs::File::create(&path).unwrap();
            for line in contents {
                writeln!(file, "{}", line).unwrap();
            }
        }
        let history = InputHistory::new_at(path);
        (dir, history)
    }

    #[test]
    fn test_new_empty() {
        let (_dir, h) = new_test_history(&[]);
        assert!(h.history().is_empty());
        assert_eq!(h.history_idx(), None);
    }

    #[test]
    fn test_append_and_load() {
        let (_dir, h) = new_test_history(&["first", "second"]);
        assert_eq!(h.history(), ["first", "second"]);
    }

    #[test]
    fn test_append_dedup() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("hello");
        h.append("hello");
        assert_eq!(h.history().len(), 1);
    }

    #[test]
    fn test_append_trim_empty() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("  ");
        h.append("");
        assert!(h.history().is_empty());
        h.append("valid");
        assert_eq!(h.history().len(), 1);
        assert_eq!(h.history()[0], "valid");
    }

    #[test]
    fn test_max_entries() {
        let (_dir, mut h) = new_test_history(&[]);
        for i in 0..1005 {
            h.append(&format!("entry_{}", i));
        }
        assert_eq!(h.history().len(), 1000);
        assert_eq!(h.history()[0], "entry_5");
        assert_eq!(h.history()[999], "entry_1004");
    }

    #[test]
    fn test_up_from_none_jumps_to_last() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("a");
        h.append("b");
        h.append("c");
        assert_eq!(h.up("draft"), Some("c".to_string()));
        assert_eq!(h.history_idx(), Some(2));
    }

    #[test]
    fn test_up_moves_older() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("a");
        h.append("b");
        h.append("c");
        h.set_history_idx(Some(2));
        assert_eq!(h.up("c"), Some("b".to_string()));
        assert_eq!(h.history_idx(), Some(1));
        assert_eq!(h.up("b"), Some("a".to_string()));
        assert_eq!(h.history_idx(), Some(0));
    }

    #[test]
    fn test_up_at_zero_does_nothing() {
        let (_dir, mut h) = new_test_history(&["only"]);
        h.set_history_idx(Some(0));
        assert_eq!(h.up("only"), None);
        assert_eq!(h.history_idx(), Some(0));
    }

    #[test]
    fn test_up_from_empty() {
        let (_dir, mut h) = new_test_history(&[]);
        assert_eq!(h.up("anything"), None);
    }

    #[test]
    fn test_down_restores_draft_from_top() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("a");
        assert_eq!(h.up("draft"), Some("a".to_string()));
        assert_eq!(h.history_idx(), Some(0));
        assert_eq!(h.down(), Some("draft".to_string()));
        assert_eq!(h.history_idx(), None);
    }

    #[test]
    fn test_down_restores_draft_from_bottom() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("a");
        assert_eq!(h.up("draft"), Some("a".to_string()));
        assert_eq!(h.history_idx(), Some(0));
        assert_eq!(h.down(), Some("draft".to_string()));
        assert_eq!(h.history_idx(), None);
    }

    #[test]
    fn test_down_from_empty() {
        let (_dir, mut h) = new_test_history(&[]);
        assert_eq!(h.down(), None);
    }

    #[test]
    fn test_up_down_cycle() {
        let (_dir, mut h) = new_test_history(&[]);
        h.append("first");
        h.append("second");
        h.append("third");

        assert_eq!(h.up("my typing"), Some("third".to_string()));
        assert_eq!(h.history_idx(), Some(2));
        assert_eq!(h.up("third"), Some("second".to_string()));
        assert_eq!(h.history_idx(), Some(1));
        assert_eq!(h.up("second"), Some("first".to_string()));
        assert_eq!(h.history_idx(), Some(0));
        assert_eq!(h.down(), Some("my typing".to_string()));
        assert_eq!(h.history_idx(), None);
    }

    #[test]
    fn test_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILENAME);

        {
            fs::write(&path, "line1\nline2\nline3\n").unwrap();
        }

        let mut h = InputHistory::new_at(path.clone());
        assert_eq!(h.history(), ["line1", "line2", "line3"]);
        h.append("line4");

        drop(h);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("line4"));
    }
}
