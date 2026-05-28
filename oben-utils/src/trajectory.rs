//! Trajectory Compressor — compresses agent conversation trajectories to fit within a token budget.
//!
//! Inspired by [`hermes-agent/trajectory_compressor.py`].
//!
//! Compression Strategy:
//! 1. Protect first turns (system, human, first gpt, first tool)
//! 2. Protect last N turns (final actions and conclusions)
//! 3. Compress the MIDDLE region — accumulating turns until enough tokens are saved
//! 4. Replace accumulated turns with a single human summary message

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const DEFAULT_TARGET_MAX_TOKENS: usize = 15250;
pub const DEFAULT_SUMMARY_TARGET_TOKENS: usize = 750;
pub const DEFAULT_PROTECT_LAST_N_TURNS: usize = 4;
pub const DEFAULT_MAX_RETRIES: usize = 3;
pub const DEFAULT_SUMMARY_NOTICE_TEXT: &str =
    "\n\nSome of your previous tool responses may be summarized to preserve context.";

pub fn token_count_heuristic(text: &str) -> usize {
    if text.is_empty() { 0 } else { text.len() / 4 }
}

pub fn count_turn_tokens(trajectory: &Value) -> Vec<usize> {
    if let Some(arr) = trajectory.as_array() {
        arr.iter()
            .map(|turn| {
                token_count_heuristic(turn.get("value").and_then(|v| v.as_str()).unwrap_or(""))
            })
            .collect()
    } else {
        Vec::new()
    }
}

pub fn count_trajectory_tokens(trajectory: &Value) -> usize {
    count_turn_tokens(trajectory).iter().sum()
}

/// Trait for generating summaries of compressed turn content.
pub trait SummaryGenerator {
    fn generate(&self, summary_input: &str, config: &CompressionConfig, turns_summary_input: &str) -> String;
}

pub struct NoopSummarizer;

impl SummaryGenerator for NoopSummarizer {
    fn generate(&self, _summary_input: &str, _config: &CompressionConfig, _turns_summary_input: &str) -> String {
        "[CONTEXT SUMMARY]: [Summary generation failed - previous turns contained tool calls and responses that have been compressed to save context space.]"
            .to_string()
    }
}

fn ensure_summary_prefix(summary: &str) -> String {
    let text = summary.trim();
    if text.starts_with("[CONTEXT SUMMARY]:") { text.to_string() }
    else if text.is_empty() { "[CONTEXT SUMMARY]: ".to_string() }
    else { format!("[CONTEXT SUMMARY]: {}", text) }
}

fn round(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn usize_max(a: usize, b: usize) -> usize { if a > b { a } else { b } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    pub tokenizer_name: String,
    pub trust_remote_code: bool,
    #[serde(default = "default_target_max_tokens")] pub target_max_tokens: usize,
    #[serde(default = "default_summary_target_tokens")] pub summary_target_tokens: usize,
    #[serde(default = "default_b")] pub protect_first_system: bool,
    #[serde(default = "default_b")] pub protect_first_human: bool,
    #[serde(default = "default_b")] pub protect_first_gpt: bool,
    #[serde(default = "default_b")] pub protect_first_tool: bool,
    #[serde(default = "default_4")] pub protect_last_n_turns: usize,
    pub summarization_model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub temperature: f64,
    #[serde(default = "default_max_retries")] pub max_retries: usize,
    #[serde(default = "default_retry_delay")] pub retry_delay: usize,
    #[serde(default = "default_b")] pub add_summary_notice: bool,
    #[serde(default = "default_notice_text")] pub summary_notice_text: String,
    pub output_suffix: String,
    pub max_concurrent_requests: usize,
    #[serde(default = "default_b")] pub skip_under_target: bool,
    #[serde(default = "default_b")] pub save_over_limit: bool,
    #[serde(default = "default_timeout")] pub per_trajectory_timeout: usize,
    #[serde(default = "default_b")] pub metrics_enabled: bool,
    #[serde(default = "default_b")] pub metrics_per_trajectory: bool,
    pub metrics_output_file: String,
}

fn default_target_max_tokens() -> usize { DEFAULT_TARGET_MAX_TOKENS }
fn default_summary_target_tokens() -> usize { DEFAULT_SUMMARY_TARGET_TOKENS }
fn default_b() -> bool { true }
fn default_4() -> usize { DEFAULT_PROTECT_LAST_N_TURNS }
fn default_max_retries() -> usize { DEFAULT_MAX_RETRIES }
fn default_retry_delay() -> usize { 2 }
fn default_timeout() -> usize { 300 }
fn default_notice_text() -> String { DEFAULT_SUMMARY_NOTICE_TEXT.to_string() }

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            tokenizer_name: "moonshotai/Kimi-K2-Thinking".to_string(),
            trust_remote_code: true,
            target_max_tokens: DEFAULT_TARGET_MAX_TOKENS,
            summary_target_tokens: DEFAULT_SUMMARY_TARGET_TOKENS,
            protect_first_system: true, protect_first_human: true,
            protect_first_gpt: true, protect_first_tool: true,
            protect_last_n_turns: DEFAULT_PROTECT_LAST_N_TURNS,
            summarization_model: "google/gemini-3-flash-preview".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            api_key_env: "OPENROUTER_API_KEY".to_string(),
            temperature: 0.3,
            max_retries: DEFAULT_MAX_RETRIES, retry_delay: 2,
            add_summary_notice: true,
            summary_notice_text: DEFAULT_SUMMARY_NOTICE_TEXT.to_string(),
            output_suffix: "_compressed".to_string(),
            max_concurrent_requests: 50,
            skip_under_target: true, save_over_limit: true,
            per_trajectory_timeout: 300,
            metrics_enabled: true, metrics_per_trajectory: true,
            metrics_output_file: "compression_metrics.json".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrajectoryMetrics {
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub tokens_saved: usize,
    pub compression_ratio: f64,
    pub original_turns: usize,
    pub compressed_turns: usize,
    pub turns_removed: usize,
    pub turns_compressed_start_idx: i32,
    pub turns_compressed_end_idx: i32,
    pub turns_in_compressed_region: usize,
    pub was_compressed: bool,
    pub still_over_limit: bool,
    pub skipped_under_target: bool,
    pub summarization_api_calls: usize,
    pub summarization_errors: usize,
}

impl TrajectoryMetrics {
    pub fn to_dict(&self) -> Value {
        serde_json::json!({
            "original_tokens": self.original_tokens,
            "compressed_tokens": self.compressed_tokens,
            "tokens_saved": self.tokens_saved,
            "compression_ratio": round(self.compression_ratio, 4),
            "original_turns": self.original_turns,
            "compressed_turns": self.compressed_turns,
            "turns_removed": self.turns_removed,
            "compression_region": {
                "start_idx": self.turns_compressed_start_idx,
                "end_idx": self.turns_compressed_end_idx,
                "turns_count": self.turns_in_compressed_region,
            },
            "was_compressed": self.was_compressed,
            "still_over_limit": self.still_over_limit,
            "skipped_under_target": self.skipped_under_target,
            "summarization_api_calls": self.summarization_api_calls,
            "summarization_errors": self.summarization_errors,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct AggregateMetrics {
    pub total_trajectories: usize,
    pub trajectories_compressed: usize,
    pub trajectories_skipped_under_target: usize,
    pub trajectories_still_over_limit: usize,
    pub trajectories_failed: usize,
    pub total_tokens_before: usize,
    pub total_tokens_after: usize,
    pub total_tokens_saved: usize,
    pub total_turns_before: usize,
    pub total_turns_after: usize,
    pub total_turns_removed: usize,
    pub total_summarization_calls: usize,
    pub total_summarization_errors: usize,
    pub compression_ratios: Vec<f64>,
    pub tokens_saved_list: Vec<usize>,
    pub turns_removed_list: Vec<usize>,
}

impl AggregateMetrics {
    pub fn new() -> Self { Self::default() }

    pub fn add_trajectory_metrics(&mut self, metrics: &TrajectoryMetrics) {
        self.total_trajectories += 1;
        self.total_tokens_before += metrics.original_tokens;
        self.total_tokens_after += metrics.compressed_tokens;
        self.total_tokens_saved += metrics.tokens_saved;
        self.total_turns_before += metrics.original_turns;
        self.total_turns_after += metrics.compressed_turns;
        self.total_turns_removed += metrics.turns_removed;
        self.total_summarization_calls += metrics.summarization_api_calls;
        self.total_summarization_errors += metrics.summarization_errors;
        if metrics.was_compressed {
            self.trajectories_compressed += 1;
            self.compression_ratios.push(metrics.compression_ratio);
            self.tokens_saved_list.push(metrics.tokens_saved);
            self.turns_removed_list.push(metrics.turns_removed);
        }
        if metrics.skipped_under_target { self.trajectories_skipped_under_target += 1; }
        if metrics.still_over_limit { self.trajectories_still_over_limit += 1; }
    }

    pub fn to_dict(&self) -> Value {
        let avg_cr = if self.compression_ratios.is_empty() { 1.0 }
            else { self.compression_ratios.iter().sum::<f64>() / self.compression_ratios.len() as f64 };
        let avg_ts = if self.tokens_saved_list.is_empty() { 0.0 }
            else { self.tokens_saved_list.iter().sum::<usize>() as f64 / self.tokens_saved_list.len() as f64 };
        let avg_tr = if self.turns_removed_list.is_empty() { 0.0 }
            else { self.turns_removed_list.iter().sum::<usize>() as f64 / self.turns_removed_list.len() as f64 };
        let td = usize_max(self.total_tokens_before, 1);
        let ts = usize_max(self.total_summarization_calls, 1);
        serde_json::json!({
            "summary": {
                "total_trajectories": self.total_trajectories,
                "trajectories_compressed": self.trajectories_compressed,
                "trajectories_skipped_under_target": self.trajectories_skipped_under_target,
                "trajectories_still_over_limit": self.trajectories_still_over_limit,
                "trajectories_failed": self.trajectories_failed,
                "compression_rate": round(self.trajectories_compressed as f64 / usize_max(self.total_trajectories, 1) as f64, 4),
            },
            "tokens": {
                "total_before": self.total_tokens_before,
                "total_after": self.total_tokens_after,
                "total_saved": self.total_tokens_saved,
                "overall_compression_ratio": round(self.total_tokens_after as f64 / td as f64, 4),
            },
            "turns": {
                "total_before": self.total_turns_before,
                "total_after": self.total_turns_after,
                "total_removed": self.total_turns_removed,
            },
            "averages": {
                "avg_compression_ratio": round(avg_cr, 4),
                "avg_tokens_saved_per_compressed": round(avg_ts, 1),
                "avg_turns_removed_per_compressed": round(avg_tr, 2),
            },
            "summarization": {
                "total_api_calls": self.total_summarization_calls,
                "total_errors": self.total_summarization_errors,
                "success_rate": round(1.0 - (self.total_summarization_errors as f64 / ts as f64), 4),
            },
        })
    }
}

fn extract_turn_content(trajectory: &Value, start: usize, end: usize) -> String {
    if let Some(ta) = trajectory.as_array() {
        (start..end.min(ta.len())).filter_map(|i| {
            let turn = &ta[i];
            let role = turn.get("from").and_then(|r| r.as_str()).unwrap_or("unknown").to_uppercase();
            let val = turn.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let mut s = val.to_string();
            if s.len() > 3000 {
                let e = s.len().saturating_sub(500);
                s = format!("{}\n...[truncated]...\n{}", &s[..1500], &s[e..]);
            }
            Some(format!("[Turn {} - {}]:\n{}", i, role, s))
        }).collect::<Vec<_>>().join("\n\n")
    } else {
        String::new()
    }
}

fn find_protected_indices(trajectory: &Value, config: &CompressionConfig) -> (HashSet<usize>, usize, usize) {
    let n = match trajectory.as_array() {
        Some(arr) => arr.len(),
        None => return (HashSet::new(), 0, 0),
    };
    if n == 0 { return (HashSet::new(), 0, 0); }

    let mut protected = HashSet::new();
    let mut first_system: Option<usize> = None;
    let mut first_human: Option<usize> = None;
    let mut first_gpt: Option<usize> = None;
    let mut first_tool: Option<usize> = None;

    let arr = trajectory.as_array().unwrap();
    for (i, turn) in arr.iter().enumerate() {
        let role = turn.get("from").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" if first_system.is_none() => first_system = Some(i),
            "human" if first_human.is_none() => first_human = Some(i),
            "gpt" if first_gpt.is_none() => first_gpt = Some(i),
            "tool" if first_tool.is_none() => first_tool = Some(i),
            _ => {}
        }
    }

    if let Some(i) = first_system { protected.insert(i); }
    if let Some(i) = first_human { protected.insert(i); }
    if let Some(i) = first_gpt { protected.insert(i); }
    if let Some(i) = first_tool { protected.insert(i); }

    let tail_start = n.saturating_sub(config.protect_last_n_turns);
    for i in tail_start..n { protected.insert(i); }

    let head_protected: Vec<usize> = protected.iter().copied().filter(|&i| i < n / 2).collect();
    let tail_protected: Vec<usize> = protected.iter().copied().filter(|&i| i >= n / 2).collect();
    let cs = head_protected.into_iter().max().unwrap_or(0).saturating_add(1);
    let ce = tail_protected.into_iter().min().unwrap_or(n);
    (protected, cs, ce)
}

pub fn compress_trajectory(
    trajectory: &Value,
    config: &CompressionConfig,
    summarizer: &dyn SummaryGenerator,
) -> (Value, TrajectoryMetrics) {
    let mut metrics = TrajectoryMetrics::default();
    let n = match trajectory.as_array() {
        Some(arr) => arr.len(),
        None => return (trajectory.clone(), metrics),
    };
    metrics.original_turns = n;

    let tt = count_turn_tokens(trajectory);
    let total: usize = tt.iter().sum();
    metrics.original_tokens = total;

    if total <= config.target_max_tokens {
        metrics.skipped_under_target = true;
        metrics.compressed_tokens = total;
        metrics.compressed_turns = n;
        metrics.compression_ratio = 1.0;
        return (trajectory.clone(), metrics);
    }

    let (_, cs, ce) = find_protected_indices(trajectory, config);
    if cs >= ce {
        metrics.compressed_tokens = total;
        metrics.compressed_turns = n;
        metrics.still_over_limit = total > config.target_max_tokens;
        return (trajectory.clone(), metrics);
    }

    let tokens_to_save = total.saturating_sub(config.target_max_tokens);
    let target = tokens_to_save.saturating_add(config.summary_target_tokens);
    let mut acc = 0usize;
    let mut until = cs;

    for i in cs..ce {
        acc = acc.saturating_add(tt[i]);
        until = i.saturating_add(1);
        if acc >= target { break; }
    }

    if acc < target && until < ce {
        until = ce;
    }

    metrics.turns_compressed_start_idx = cs as i32;
    metrics.turns_compressed_end_idx = until as i32;
    metrics.turns_in_compressed_region = until.saturating_sub(cs);

    let content = extract_turn_content(trajectory, cs, until);
    let summary = ensure_summary_prefix(&summarizer.generate(&content, config, &content));

    let arr = trajectory.as_array().unwrap();
    let mut compressed: Vec<Value> = Vec::new();

    for i in 0..cs {
        let turn = arr[i].clone();
        let turn_from = turn.get("from").and_then(|r| r.as_str());
        if turn_from == Some("system") && config.add_summary_notice {
            let mut t = turn.clone();
            if let Some(v) = t.get_mut("value") {
                if let Some(s) = v.as_str() {
                    *v = serde_json::json!(format!("{}{}", s, config.summary_notice_text));
                }
            }
            compressed.push(t);
        } else {
            compressed.push(turn);
        }
    }

    compressed.push(serde_json::json!({"from": "human", "value": summary}));
    for i in until..n {
        compressed.push(arr[i].clone());
    }

    let cv = serde_json::Value::Array(compressed.clone());
    metrics.compressed_turns = compressed.len();
    metrics.compressed_tokens = count_trajectory_tokens(&cv);
    metrics.turns_removed = metrics.original_turns.saturating_sub(metrics.compressed_turns);
    metrics.tokens_saved = metrics.original_tokens.saturating_sub(metrics.compressed_tokens);
    metrics.compression_ratio = metrics.compressed_tokens as f64 / usize_max(metrics.original_tokens, 1) as f64;
    metrics.was_compressed = true;
    metrics.still_over_limit = metrics.compressed_tokens > config.target_max_tokens;

    (serde_json::Value::Array(compressed), metrics)
}

pub fn process_entry(
    entry: &Value,
    config: &CompressionConfig,
    summarizer: &dyn SummaryGenerator,
) -> (Value, TrajectoryMetrics) {
    let mut metrics = TrajectoryMetrics::default();
    let conv = match entry.get("conversations") { Some(c) => c, None => return (entry.clone(), metrics) };
    let (compressed, m) = compress_trajectory(conv, config, summarizer);
    metrics = m;
    let mut result = entry.clone();
    if let Some(obj) = result.as_object_mut() {
        obj.insert("conversations".to_string(), compressed);
        if config.metrics_per_trajectory && metrics.was_compressed {
            obj.insert("compression_metrics".to_string(), metrics.to_dict());
        }
    }
    (result, metrics)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Given a trajectory with standard roles, When find_protected_indices runs,
    /// Then returns correct protected set and compressible range.
    #[test]
    fn test_find_protected_indices_basic() {
        let traj: Value = serde_json::json!([
            {"from": "system", "value": "helpful"},
            {"from": "human", "value": "hello"},
            {"from": "gpt", "value": "hi"},
            {"from": "tool", "value": "res1"},
            {"from": "gpt", "value": "mid"},
            {"from": "tool", "value": "res2"},
            {"from": "gpt", "value": "mid2"},
            {"from": "tool", "value": "res3"},
        ]);

        let cfg = CompressionConfig::default();
        let (protected, start, end) = find_protected_indices(&traj, &cfg);

        assert!(protected.contains(&0)); assert!(protected.contains(&1));
        assert!(protected.contains(&2)); assert!(protected.contains(&3));
        // Last 4 of 8 are also protected: 4,5,6,7
        assert!(protected.contains(&4)); assert!(protected.contains(&5));
        assert!(protected.contains(&6)); assert!(protected.contains(&7));
        assert!(start >= end);
    }

    /// Given a long trajectory, When find_protected_indices runs,
    /// Then the compressible region covers middle turns.
    #[test]
    fn test_find_protected_indices_with_compression_region() {
        let mut turns = Vec::new();
        turns.push(serde_json::json!({"from": "system", "value": "s"}));
        turns.push(serde_json::json!({"from": "human", "value": "h"}));
        turns.push(serde_json::json!({"from": "gpt", "value": "g"}));
        turns.push(serde_json::json!({"from": "tool", "value": "t"}));
        for i in 0..20 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("mid{}", i)}));
            turns.push(serde_json::json!({"from": "tool", "value": format!("res{}", i)}));
        }
        turns.push(serde_json::json!({"from": "gpt", "value": "end1"}));
        turns.push(serde_json::json!({"from": "tool", "value": "end2"}));

        let cfg = CompressionConfig::default();
        let (protected, start, end) = find_protected_indices(&serde_json::json!(turns), &cfg);

        assert_eq!(protected.len(), 8);
        assert!(start >= 4);
        let n = turns.len();
        assert_eq!(end, n - 4);
        assert!(start < end);
    }

    #[test]
    fn test_find_protected_indices_empty() {
        let cfg = CompressionConfig::default();
        let (p, s, e) = find_protected_indices(&serde_json::json!([]), &cfg);
        assert!(p.is_empty()); assert_eq!(s, 0); assert_eq!(e, 0);
    }

    /// Given few turns fewer than protect_last_n_turns, When find_protected_indices runs,
    /// Then all turns are protected.
    #[test]
    fn test_find_protected_indices_fewer_than_last_n() {
        let cfg = CompressionConfig::default();
        let traj: Value = serde_json::json!([
            {"from": "system", "value": "s"},
            {"from": "human", "value": "h"},
            {"from": "gpt", "value": "g"},
        ]);
        let (p, s, e) = find_protected_indices(&traj, &cfg);
        assert_eq!(p.len(), 3);
        assert!(s >= e);
    }

    /// Given an over-limit trajectory, When compress_trajectory runs with NoopSummarizer,
    /// Then returns compressed trajectory with summary placeholder.
    #[test]
    fn test_compress_trajectory_over_limit() {
        let cfg = CompressionConfig::default();
        let mut turns = Vec::new();
        turns.push(serde_json::json!({"from": "system", "value": "helpful assistant"}));
        turns.push(serde_json::json!({"from": "human", "value": "hello"}));
        turns.push(serde_json::json!({"from": "gpt", "value": "hi"}));
        turns.push(serde_json::json!({"from": "tool", "value": "tool result"}));

        let long_val = "x".repeat(70000);
        turns.push(serde_json::json!({"from": "gpt", "value": long_val}));
        for i in 0..200 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("turn{}", i)}));
            turns.push(serde_json::json!({"from": "tool", "value": format!("result{}", i)}));
        }

        let traj: Value = serde_json::json!(turns);
        let total = count_trajectory_tokens(&traj);
        assert!(total > DEFAULT_TARGET_MAX_TOKENS, "Test setup: needed >{} tokens but got {}", DEFAULT_TARGET_MAX_TOKENS, total);

        let ns = NoopSummarizer;
        let (compressed, m) = compress_trajectory(&traj, &cfg, &ns);

        assert!(m.was_compressed);
        assert!(!m.skipped_under_target);
        assert!(m.tokens_saved > 0);
        assert!(m.compression_ratio < 1.0);
        assert!(compressed.as_array().unwrap().len() <= traj.as_array().unwrap().len());

        let summary_found = compressed.as_array().unwrap().iter().any(|t| {
            t.get("value").and_then(|v| v.as_str()).unwrap_or("").starts_with("[CONTEXT SUMMARY]:")
        });
        assert!(summary_found);
    }

    /// Given an under-limit trajectory, When compress_trajectory runs,
    /// Then trajectory is returned unchanged with skipped_under_target=true.
    #[test]
    fn test_compress_trajectory_under_limit() {
        let cfg = CompressionConfig::default();
        let traj: Value = serde_json::json!([
            {"from": "system", "value": "short"},
            {"from": "human", "value": "hi"},
            {"from": "gpt", "value": "hello"},
        ]);
        let ns = NoopSummarizer;
        let (compressed, m) = compress_trajectory(&traj, &cfg, &ns);

        assert!(m.skipped_under_target);
        assert!(!m.was_compressed);
        assert_eq!(m.compression_ratio, 1.0);
        assert_eq!(compressed, traj);
    }

    /// Given a JSONL entry, When process_entry runs,
    /// Then it returns a new entry with compressed conversations and metrics metadata.
    #[test]
    fn test_process_entry_with_metadata() {
        let cfg = CompressionConfig::default();
        let long_val = "y".repeat(64000);
        let mut turns = Vec::new();
        turns.push(serde_json::json!({"from": "system", "value": "s"}));
        turns.push(serde_json::json!({"from": "human", "value": "h"}));
        turns.push(serde_json::json!({"from": "gpt", "value": "g"}));
        turns.push(serde_json::json!({"from": "tool", "value": "t"}));
        for i in 0..10 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("mid{}", i)}));
        }
        turns.push(serde_json::json!({"from": "gpt", "value": long_val}));
        for i in 10..20 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("later{}", i)}));
        }

        let entry: Value = serde_json::json!({"id": "test-entry-1", "conversations": turns});
        let ns = NoopSummarizer;
        let (result, m) = process_entry(&entry, &cfg, &ns);

        assert!(m.was_compressed);
        let robj = result.as_object().unwrap();
        assert!(robj.contains_key("compression_metrics"));
        let cm = robj.get("compression_metrics").unwrap();
        assert_eq!(cm["original_tokens"], m.original_tokens);
        assert_eq!(cm["was_compressed"], true);
    }

    /// Given an entry without conversations, When process_entry runs,
    /// Then it returns the entry unchanged with no metrics.
    #[test]
    fn test_process_entry_no_conversations() {
        let cfg = CompressionConfig::default();
        let entry: Value = serde_json::json!({"id": "no-convo", "other": "field"});
        let ns = NoopSummarizer;
        let (result, m) = process_entry(&entry, &cfg, &ns);

        assert_eq!(result, entry);
        assert_eq!(m.original_tokens, 0);
        assert!(!m.was_compressed);
    }

    /// Given aggregate metrics with multiple trajectories, When to_dict runs,
    /// Then the output has correct structural keys and totals.
    #[test]
    fn test_aggregate_metrics_accumulation() {
        let mut agg = AggregateMetrics::new();
        agg.add_trajectory_metrics(&TrajectoryMetrics {
            original_tokens: 1000, compressed_tokens: 500, tokens_saved: 500,
            compression_ratio: 0.5, original_turns: 10, compressed_turns: 5,
            turns_removed: 5, was_compressed: true, summarization_api_calls: 1, ..Default::default()
        });
        agg.add_trajectory_metrics(&TrajectoryMetrics {
            original_tokens: 200, compressed_tokens: 200,
            skipped_under_target: true, ..Default::default()
        });

        assert_eq!(agg.total_trajectories, 2);
        assert_eq!(agg.trajectories_compressed, 1);
        assert_eq!(agg.trajectories_skipped_under_target, 1);
        assert_eq!(agg.total_tokens_before, 1200);
        assert_eq!(agg.total_tokens_after, 700);

        let d = agg.to_dict();
        assert_eq!(d["summary"]["total_trajectories"], 2);
        assert_eq!(d["tokens"]["total_before"], 1200);
        assert_eq!(d["tokens"]["total_after"], 700);
    }

    /// Given aggregate metrics with multiple compressed trajectories, When to_dict runs,
    /// Then averages are correctly computed.
    #[test]
    fn test_aggregate_metrics_averages() {
        let mut agg = AggregateMetrics::new();
        for _ in 0..2 {
            agg.add_trajectory_metrics(&TrajectoryMetrics {
                original_tokens: 1000, compressed_tokens: 500, tokens_saved: 500,
                compression_ratio: 0.5, original_turns: 10, compressed_turns: 5,
                turns_removed: 5, was_compressed: true, ..Default::default()
            });
        }

        let d = agg.to_dict();
        let avg = d["averages"].as_object().unwrap();
        assert_eq!(avg["avg_compression_ratio"], 0.5);
        assert_eq!(avg["avg_tokens_saved_per_compressed"], 500.0);
        assert_eq!(avg["avg_turns_removed_per_compressed"], 5.0);
    }

    /// Given invalid input, When compress_trajectory runs,
    /// Then it gracefully returns unchanged.
    #[test]
    fn test_compress_trajectory_invalid_object() {
        let cfg = CompressionConfig::default();
        let obj: Value = serde_json::json!({"not": "an array"});
        let ns = NoopSummarizer;
        let (compressed, m) = compress_trajectory(&obj, &cfg, &ns);
        assert_eq!(compressed, obj);
        assert!(!m.was_compressed);
    }

    /// Given an empty array, When compress_trajectory runs,
    /// Then it returns it unchanged.
    #[test]
    fn test_compress_trajectory_empty_array() {
        let cfg = CompressionConfig::default();
        let traj: Value = serde_json::json!([]);
        let ns = NoopSummarizer;
        let (compressed, m) = compress_trajectory(&traj, &cfg, &ns);
        assert_eq!(compressed, traj);
        assert!(!m.was_compressed);
        assert!(m.skipped_under_target);
    }

    /// Given add_summary_notice=true, When compress_trajectory runs,
    /// Then the system message in the compressed output has the notice appended.
    #[test]
    fn test_compress_adds_summary_notice() {
        let mut cfg = CompressionConfig::default();
        cfg.add_summary_notice = true;
        cfg.summary_notice_text = " [NOTICE: summarized]".to_string();
        let mut turns = Vec::new();
        turns.push(serde_json::json!({"from": "system", "value": "You are a helpful assistant"}));
        turns.push(serde_json::json!({"from": "human", "value": "hi"}));
        turns.push(serde_json::json!({"from": "gpt", "value": "hello"}));
        turns.push(serde_json::json!({"from": "tool", "value": "res"}));
        for i in 0..10 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("mid{}", i)}));
        }
        let long_val = "z".repeat(64000);
        turns.push(serde_json::json!({"from": "gpt", "value": long_val}));
        for i in 10..20 {
            turns.push(serde_json::json!({"from": "gpt", "value": format!("later{}", i)}));
        }

        let traj: Value = serde_json::json!(turns);
        let ns = NoopSummarizer;
        let (compressed, _m) = compress_trajectory(&traj, &cfg, &ns);

        let sys = compressed.as_array().unwrap().iter()
            .find(|t| t.get("from").and_then(|r| r.as_str()) == Some("system"))
            .expect("System turn should exist");

        let val = sys.get("value").and_then(|v| v.as_str()).unwrap_or("");
        assert!(val.contains("[NOTICE: summarized]"));
    }

    /// Given a trajectory with standard roles only, When compress_trajectory runs,
    /// Then last 4 are also protected so there's no compressible region.
    #[test]
    fn test_find_protected_indices_no_standard_roles() {
        let cfg = CompressionConfig::default();
        let traj: Value = serde_json::json!([
            {"from": "user", "value": "a"},
            {"from": "assistant", "value": "b"},
            {"from": "user", "value": "c"},
            {"from": "assistant", "value": "d"},
            {"from": "assistant", "value": "e"},
            {"from": "assistant", "value": "f"},
            {"from": "assistant", "value": "g"},
            {"from": "assistant", "value": "h"},
        ]);

        let (protected, start, end) = find_protected_indices(&traj, &cfg);
        assert!(protected.contains(&4));
        assert!(protected.contains(&5));
        assert!(protected.contains(&6));
        assert!(protected.contains(&7));
        // No standard roles found, so only last 4 are protected
        // head_protected is empty, so start = 0 + 1 = 1
        assert_eq!(start, 1);
        assert_eq!(end, 4);
    }
}
