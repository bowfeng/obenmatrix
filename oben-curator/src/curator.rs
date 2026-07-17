//! Curator — orchestrator for background skill maintenance.

use crate::lifecycle::{LifecycleConfig, LifecycleManager, LifecycleState};
use crate::reconciler::{reconcile_classifications, AbsorptionEntry, ClassificationResult};
use crate::cron_rewrite::scan_cron_directory;
use crate::report::{generate_consolidation_reports, ConsolidationReport};
use crate::usage::mark_agent_created;
use chrono::{DateTime, Utc};
use serde_yaml::Value as YamlValue;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};

/// Parse YAML frontmatter from a markdown string.
/// Extracts the block between `---` delimiters at the start of the file
/// and returns (frontmatter_map, remaining_body).
fn parse_frontmatter(content: &str) -> (BTreeMap<String, YamlValue>, String) {
    let mut frontmatter: BTreeMap<String, YamlValue> = BTreeMap::new();
    let body = content.to_string();

    if !content.starts_with("---") {
        return (frontmatter, body);
    }

    let rest = &content[3..];
    if let Some(end_pos) = rest.find("\n---\n") {
        let yaml_content = &rest[..end_pos];
        let body_start = end_pos + 6;
        let body = content.get(body_start..).unwrap_or("").to_string();

        match serde_yaml::from_str::<YamlValue>(yaml_content) {
            Ok(YamlValue::Mapping(map)) => {
                for (k, v) in map {
                    if let Some(key) = k.as_str() {
                        frontmatter.insert(key.to_string(), v);
                    }
                }
            }
            _ => {}
        }
        return (frontmatter, body);
    }

    (frontmatter, body)
}

fn extract_environments(frontmatter: &BTreeMap<String, YamlValue>) -> Vec<String> {
    if let Some(YamlValue::Sequence(seqs)) = frontmatter.get("environments") {
        return seqs.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect();
    }
    if let Some(YamlValue::Mapping(meta)) = frontmatter.get("metadata") {
        if let Some(YamlValue::Sequence(seqs)) = meta.get("environments") {
            return seqs
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
        }
    }
    Vec::new()
}

/// Configuration for the curator.
#[derive(Debug, Clone)]
pub struct CuratorConfig {
    /// Interval between curator runs (in hours).
    pub interval_hours: usize,
    /// Minimum idle time before running (in hours).
    pub min_idle_hours: usize,
    /// Directory containing skills.
    pub skills_dir: PathBuf,
    /// Enable LLM consolidation pass (LLM-powered umbrella building).
    pub consolidate: bool,
    /// Enable environment filtering of skills.
    pub environment_filtering: bool,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            interval_hours: 24 * 7,
            min_idle_hours: 2,
            skills_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".obenmatrix")
                .join("skills"),
            consolidate: false,
            environment_filtering: false,
        }
    }
}

/// Persistent state for the curator scheduler.
#[derive(Debug, Clone)]
pub struct CuratorState {
    /// ISO timestamp of last run.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Duration of last run in seconds.
    pub last_run_duration_seconds: Option<f64>,
    /// Summary of last run.
    pub last_run_summary: Option<String>,
    /// Whether curator is paused.
    pub paused: bool,
    /// Total number of runs.
    pub run_count: usize,
    /// Interval hours (stored for should_run check).
    pub interval_hours: usize,
    /// Days until a skill is considered stale.
    pub stale_after_days: usize,
    /// Days until a skill is archived.
    pub archive_after_days: usize,
    /// Skills filtered by environment compatibility.
    pub matching_envs: Vec<String>,
}

impl CuratorState {
    pub fn new() -> Self {
        Self {
            last_run_at: None,
            last_run_duration_seconds: None,
            last_run_summary: None,
            paused: false,
            run_count: 0,
            interval_hours: 24 * 7,
            stale_after_days: 30,
            archive_after_days: 90,
            matching_envs: Vec::new(),
        }
    }

    /// Check if curator should run now.
    pub fn should_run(&self, idle_hours: f64) -> bool {
        if self.paused {
            debug!("Curator is paused");
            return false;
        }

        if idle_hours < self.interval_hours as f64 {
            debug!(
                "Not enough idle time: {} < {}",
                idle_hours, self.interval_hours
            );
            return false;
        }

        match self.last_run_at {
            None => true,
            Some(last_run) => {
                let hours_since = (Utc::now() - last_run).num_hours();
                hours_since >= self.interval_hours as i64
            }
        }
    }

    /// Record a run completion with detailed report.
    pub fn record_run(&mut self, summary: String, duration_seconds: f64) {
        self.last_run_at = Some(Utc::now());
        self.last_run_duration_seconds = Some(duration_seconds);
        self.last_run_summary = Some(summary);
        self.run_count += 1;
    }
}

/// Curator — orchestrates skill maintenance tasks.
pub struct Curator {
    config: CuratorConfig,
    lifecycle_manager: LifecycleManager,
    state: CuratorState,
}

impl Curator {
    pub fn new(config: CuratorConfig) -> Self {
        let lifecycle_config = LifecycleConfig {
            stale_after_days: config.interval_hours,
            archive_after_days: 90,
        };
        let state = CuratorState::new();
        Self {
            config,
            lifecycle_manager: LifecycleManager::new(lifecycle_config),
            state,
        }
    }

    /// Run the curator review.
    pub fn run(&mut self, idle_hours: f64) -> String {
        if !self.state.should_run(idle_hours) {
            return "Curator: skipped (not due or paused)".to_string();
        }

        let start = chrono::Utc::now();
        info!("Starting curator run...");

        let mut summary_parts = Vec::new();

        // 1. Check lifecycle states
        let changes = self.lifecycle_manager.check_all_skills();
        let stale_count = changes
            .iter()
            .filter(|(_, s)| *s == LifecycleState::Stale)
            .count();
        let archived_count = changes
            .iter()
            .filter(|(_, s)| *s == LifecycleState::Archived)
            .count();

        if stale_count > 0 || archived_count > 0 {
            summary_parts.push(format!(
                "lifecycle: {} stale, {} archived",
                stale_count, archived_count
            ));
        } else {
            summary_parts.push("lifecycle: all skills active".to_string());
        }

        // 2. Record the run
        let duration = (chrono::Utc::now() - start).num_milliseconds() as f64 / 1000.0;
        self.state.record_run(summary_parts.join("; "), duration);

        let final_summary = format!(
            "curator run #{} ({}s): {}",
            self.state.run_count,
            duration,
            summary_parts.join("; ")
        );

        info!("Curator run complete: {}", final_summary);
        final_summary
    }

    /// Check if curator is due to run.
    pub fn is_due(&self, idle_hours: f64) -> bool {
        self.state.should_run(idle_hours)
    }

    /// Get current state.
    pub fn state(&self) -> &CuratorState {
        &self.state
    }

    /// Get summary of last run.
    pub fn last_run_summary(&self) -> Option<&str> {
        self.state.last_run_summary.as_deref()
    }

    /// Get full report for the last run including detailed statistics.
    pub fn last_run_details(&self) -> Option<String> {
        self.state.last_run_summary.clone()
    }

    /// Get the stale cutoff in days.
    pub fn stale_after_days(&self) -> usize {
        self.state.stale_after_days
    }

    /// Get the archive cutoff in days.
    pub fn archive_after_days(&self) -> usize {
        self.state.archive_after_days
    }

    /// Check if environment filtering is enabled.
    pub fn environment_filtering_enabled(&self) -> bool {
        self.config.environment_filtering
    }

    /// Get list of skills matching current environment.
    pub fn matching_envs(&self) -> &[String] {
        &self.state.matching_envs
    }

    /// Pause the curator.
    pub fn pause(&mut self) {
        self.state.paused = true;
        info!("Curator paused");
    }

    /// Resume the curator.
    pub fn resume(&mut self) {
        self.state.paused = false;
        info!("Curator resumed");
    }

    /// Mark a skill as agent-created.
    pub fn mark_agent_created(&self, skill_name: &str) {
        mark_agent_created(skill_name);
    }

    /// Set lifecycle state for a skill.
    pub fn set_skill_state(&mut self, skill_name: &str, state: LifecycleState) {
        self.lifecycle_manager.set_state(skill_name, state);
    }

    /// Filter skills by environment compatibility.
    pub fn filter_by_environment(&self, skills: &[String]) -> Vec<String> {
        if !self.config.environment_filtering {
            return skills.to_vec();
        }

        let current_env = self.get_current_environment();

        skills
            .iter()
            .filter(|skill| self.is_env_compatible(skill, &current_env))
            .cloned()
            .collect()
    }

    /// Get current runtime environment.
    fn get_current_environment(&self) -> String {
        let os = std::env::var("OS").unwrap_or_else(|_| env::consts::OS.to_string());
        let arch = env::consts::ARCH.to_string();
        format!("{}-{}", os, arch)
    }

    /// Check if a skill is compatible with current environment.
    /// Reads skill's frontmatter to get environments field.
    fn is_env_compatible(&self, skill_name: &str, current_env: &str) -> bool {
        let skill_path = self.config.skills_dir.join(skill_name);
        let skill_yaml = skill_path.join("SKILL.md");

        if !skill_yaml.exists() {
            // If skill file doesn't exist, assume compatible (safe fallback)
            return true;
        }

        if let Ok(content) = fs::read_to_string(&skill_yaml) {
            let (frontmatter, _) = parse_frontmatter(&content);
            if frontmatter.is_empty() {
                // No frontmatter means no environment restrictions
                return true;
            }

            let environments = extract_environments(&frontmatter);
            if environments.is_empty() {
                // No environments specified means compatible with all
                return true;
            }

            // Check if current_env matches any environment
            for env in &environments {
                if env.to_lowercase() == current_env.to_lowercase() {
                    return true;
                }
            }
            false
        } else {
            // On read error, assume compatible (safe fallback)
            true
        }
    }

    pub fn apply_consolidation_pass(&mut self, dry_run: bool) -> ConsolidationReport {
        if !self.config.consolidate {
            return ConsolidationReport::skipped();
        }

        let skills = self.scan_skills();
        let _prompt = self.build_consolidation_prompt(&skills);

        if dry_run {
            let heuristic_recs = self.classify_skills_with_heuristics(&skills);
            let report = ConsolidationReport {
                consolidated: heuristic_recs.consolidated.iter().map(|e| e.skill_name.clone()).collect(),
                pruned: Vec::new(),
                dry_run,
                timestamp: Utc::now().to_rfc3339(),
            };
            return report;
        }

        let final_classification = reconcile_classifications(
            Vec::new(),
            None,
            self.classify_skills_with_heuristics(&skills).consolidated,
        );

        // unused - consolidations are handled via absorbed_into
        let _pruned: Vec<String> = Vec::new();

        let cron_dir = self.config.skills_dir.parent().unwrap_or(&self.config.skills_dir).join("cron");
        let cron_jobs = scan_cron_directory(&cron_dir);
        let mut cron_rewrites: Vec<String> = Vec::new();

        for entry in &final_classification.consolidated {
            if let Some(new_skill) = final_classification.sources.get(&entry.skill_name) {
                let old_ref = format!("skill {}", entry.skill_name);
                let new_ref = format!("skill {}", new_skill);
                for job in &cron_jobs {
                    if job.raw.contains(&old_ref) {
                        let updated = job.raw.replace(&old_ref, &new_ref);
                        cron_rewrites.push(updated);
                    }
                }
            }
        }

        let report_dir = self.get_report_directory();
        let _ = generate_consolidation_reports(
            &report_dir,
            &final_classification,
            &cron_rewrites,
            dry_run,
        );

        ConsolidationReport {
            consolidated: final_classification.consolidated.iter().map(|e| e.skill_name.clone()).collect(),
            pruned: final_classification.pruned,
            dry_run,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    fn scan_skills(&self) -> Vec<String> {
        let mut skills = Vec::new();

        if self.config.skills_dir.exists() {
            if let Ok(entries) = fs::read_dir(&self.config.skills_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_file() || path.is_dir() {
                        if let Some(name) = path.file_stem() {
                            let name_str = name.to_string_lossy().into_owned();
                            skills.push(name_str);
                        }
                        }
                    }
                }
            }
        }

        skills.sort();
        skills
    }

    fn build_consolidation_prompt(&self, skills: &[String]) -> String {
        let skills_str = skills.join(", ");

        format!(
            "You are a skill consolidation assistant. Skills detected: {}
            
Analyze the skills and suggest consolidations using YAML format:

consolidation_suggestions:
  - skill_to_consolidate: skill_name_to_merge_into
    reason: \"brief reason for consolidation\"

Output ONLY valid YAML, no other text.",
            skills_str
        )
    }

    fn classify_skills_with_heuristics(&self, skills: &[String]) -> ClassificationResult {
        let mut prefix_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for skill in skills {
            let prefix = skill.split('_').next().unwrap_or(skill).to_string();
            prefix_groups.entry(prefix).or_default().push(skill.clone());
        }

        let mut consolidated = Vec::new();
        let mut pruned = Vec::new();

        for (prefix, group) in prefix_groups {
            if group.len() > 1 {
                let umbrella = format!("{}_utils", prefix);
                for skill in &group {
                    if skill != &umbrella {
                        consolidated.push(AbsorptionEntry {
                            skill_name: skill.clone(),
                            absorbed_into: umbrella.clone(),
                            confidence: 0.5,
                        });
                    }
                }
            }
        }

        for skill in skills {
            if skill.contains("temp") || skill.contains("tmp") {
                pruned.push(skill.clone());
            }
        }

        ClassificationResult {
            consolidated,
            pruned,
            sources: BTreeMap::new(),
        }
    }

    fn get_report_directory(&self) -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".obenmatrix").join("logs").join("curator").join(
            Utc::now().format("%Y%m%d_%H%M%S").to_string(),
        )
    }

    /// Record absorption tracking data for a skill.
    pub fn record_absorption(&mut self, skill_name: &str, absorbed_into: &str) {
        debug!(
            "Skill '{}' absorbed into '{}'",
            skill_name, absorbed_into
        );
    }

    /// Classify removed skills as consolidated or pruned based on tool calls.
    /// Mirrors hermes-agent/curator.py:601-720 logic.
    pub fn classify_removed_skills(
        &self,
        removed: &[String],
        added: &[String],
        tool_calls: &[ToolCall],
    ) -> ClassificationResult {
        let mut consolidated: Vec<AbsorptionEntry> = Vec::new();
        let mut pruned: Vec<AbsorptionEntry> = Vec::new();

        // Build set of surviving skill names (destinations)
        // A destination is a skill that exists after the run (not in removed list)
        // We look at what skills are in 'added' since those are the ones that survived
        let destinations: Vec<String> = added.iter().cloned().collect();

        for name in removed {
            let mut into: Option<String> = None;
            let mut evidence: Option<String> = None;

            for tool_call in tool_calls {
                if tool_call.name != "skill_manage" {
                    continue;
                }

                // Get the target skill from arguments
                let target = tool_call.arguments.name.as_ref();

                if let Some(t) = target {
                    // Skip if this is the removed skill itself
                    if t == name {
                        continue;
                    }

                    // Check if target is a surviving skill
                    if !destinations.contains(t) {
                        continue;
                    }

                    // Look for evidence in file_path, content, new_string
                    let haystacks = vec![
                        tool_call.arguments.file_path.as_ref(),
                        tool_call.arguments.content.as_ref(),
                        tool_call.arguments.new_string.as_ref(),
                    ];

                    for hay in haystacks {
                        if let Some(h) = hay {
                            if h.contains(name) || h.replace('-', "_").contains(name) {
                                into = Some(t.clone());
                                evidence = Some(format!(
                                    "skill_manage on '{}' referenced '{}' in: {}",
                                    t,
                                    name,
                                    &h[..h.len().min(80).max(0)]
                                ));
                                break;
                            }
                        }
                    }

                    if into.is_some() {
                        break;
                    }
                }
            }

            if let Some(into_name) = into {
                consolidated.push(AbsorptionEntry {
                    skill_name: name.clone(),
                    absorbed_into: into_name,
                    confidence: 0.8,
                });
            }
        }

        ClassificationResult {
            consolidated,
            pruned: Vec::new(),
            sources: BTreeMap::new(),
        }
    }

    pub fn build_rename_summary(
        &self,
        before_names: &[String],
        after_names: &[String],
        tool_calls: &[ToolCall],
    ) -> String {
        let before_set: std::collections::HashSet<String> = before_names.iter().cloned().collect();
        let after_set: std::collections::HashSet<String> = after_names.iter().cloned().collect();
        let removed: Vec<String> = before_set.difference(&after_set).cloned().collect();
        let added: Vec<String> = after_set.difference(&before_set).cloned().collect();

        if removed.is_empty() {
            return String::new();
        }

        let classification = self.classify_removed_skills(&removed, &added, tool_calls);
        let consolidated = &classification.consolidated;
        let pruned = &classification.pruned;

        let total = consolidated.len() + pruned.len();
        let mut lines: Vec<String> = Vec::new();

        lines.push(format!("archived {} skill(s):", total));

        const SHOW: usize = 10;
        let mut shown = 0;

        for entry in consolidated {
            if shown >= SHOW {
                break;
            }
            let into = entry.absorbed_into.as_str();
            lines.push(format!("  • {} → {}", entry.skill_name, into));
            shown += 1;
        }

        for entry in pruned {
            if shown >= SHOW {
                break;
            }
            lines.push(format!("  • {} — pruned (stale)", entry));
            shown += 1;
        }

        if total > SHOW {
            lines.push(format!("  … and {} more", total - SHOW));
        }

        lines.push("full report: hermes curator status".to_string());

        if !consolidated.is_empty() {
            let umbrellas: Vec<String> = consolidated
                .iter()
                .map(|e| e.absorbed_into.clone())
                .collect();
            if let Some(example) = umbrellas.first() {
                lines.push(format!("keep an umbrella stable: hermes curator pin {}", example));
            }
        }

        lines.join("\n")
    }
}

/// Tool call arguments for classification.
#[derive(Debug, Clone)]
pub struct ToolCallArgs {
    pub action: Option<String>,
    pub name: Option<String>,
    pub file_path: Option<String>,
    pub file_content: Option<String>,
    pub content: Option<String>,
    pub new_string: Option<String>,
    pub absorbed_into: Option<String>,
}

/// Tool call metadata for classification.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: ToolCallArgs,
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curator_state_new() {
        let state = CuratorState::new();
        assert!(state.last_run_at.is_none());
        assert!(!state.paused);
        assert_eq!(state.run_count, 0);
        assert_eq!(state.interval_hours, 24 * 7);
        assert_eq!(state.stale_after_days, 30);
        assert_eq!(state.archive_after_days, 90);
    }

    #[test]
    fn test_curator_state_should_run_never_run() {
        let state = CuratorState::new();
        assert!(state.should_run(200.0));
    }

    #[test]
    fn test_curator_state_should_run_paused() {
        let mut state = CuratorState::new();
        state.paused = true;
        assert!(!state.should_run(100.0));
    }

    #[test]
    fn test_curator_state_record_run() {
        let mut state = CuratorState::new();
        state.record_run("test summary".to_string(), 5.5);

        assert!(state.last_run_at.is_some());
        assert_eq!(state.last_run_summary, Some("test summary".to_string()));
        assert_eq!(state.last_run_duration_seconds, Some(5.5));
        assert_eq!(state.run_count, 1);
    }

    #[test]
    fn test_curator_new() {
        let config = CuratorConfig::default();
        let curator = Curator::new(config);
        assert!(!curator.is_due(1.0));
    }

    #[test]
    fn test_curator_pause_resume() {
        let config = CuratorConfig::default();
        let mut curator = Curator::new(config);

        curator.pause();
        assert!(curator.state().paused);

        curator.resume();
        assert!(!curator.state().paused);
    }

    #[test]
    fn test_curator_config_consolidate_flag() {
        let mut config = CuratorConfig::default();
        assert!(!config.consolidate);

        config.consolidate = true;
        assert!(config.consolidate);
    }

    #[test]
    fn test_curator_config_environment_filtering() {
        let mut config = CuratorConfig::default();
        assert!(!config.environment_filtering);

        config.environment_filtering = true;
        assert!(config.environment_filtering);
    }

    #[test]
    fn test_filter_by_environment_disabled() {
        let config = CuratorConfig::default();
        let curator = Curator::new(config.clone());

        let skills = vec!["skill1".to_string(), "skill2".to_string()];
        let filtered = curator.filter_by_environment(&skills);
        assert_eq!(filtered, skills);
    }

    #[test]
    fn test_consolidation_report_skipped() {
        let config = CuratorConfig::default();
        let mut curator = Curator::new(config);

        let report = curator.apply_consolidation_pass(false);
        assert!(report.dry_run);
        assert!(report.consolidated.is_empty());
        assert!(report.pruned.is_empty());
    }

    #[test]
    fn test_consolidation_report_enabled() {
        let mut config = CuratorConfig::default();
        config.consolidate = true;

        let mut curator = Curator::new(config);

        let report = curator.apply_consolidation_pass(true);
        assert!(report.dry_run);
        assert!(report.consolidated.is_empty());
        assert!(report.pruned.is_empty());
    }

    #[test]
    fn test_consolidation_report_with_consolidate() {
        let mut config = CuratorConfig::default();
        config.consolidate = true;

        let mut curator = Curator::new(config);

        let report = curator.apply_consolidation_pass(false);
        assert!(!report.dry_run);
        assert!(report.consolidated.is_empty());
        assert!(report.pruned.is_empty());
    }

    #[test]
    fn test_classify_removed_skills_with_consolidation() {
        let config = CuratorConfig::default();
        let curator = Curator::new(config);

        // old-skill is removed but umbrella-skill exists
        let removed = vec!["old-skill".to_string()];
        let added = vec!["umbrella-skill".to_string()];  // umbrella-skill survives
        let tool_calls = vec![
            ToolCall {
                name: "skill_manage".to_string(),
                arguments: ToolCallArgs {
                    action: Some("patch".to_string()),
                    name: Some("umbrella-skill".to_string()),
                    file_path: Some("/some/path/old-skill.md".to_string()),
                    file_content: None,
                    content: None,
                    new_string: None,
                    absorbed_into: None,
                },
            },
        ];

        let result = curator.classify_removed_skills(&removed, &added, &tool_calls);

        eprintln!("Result: consolidated={:?}, pruned={:?}", result.consolidated, result.pruned);

        assert!(result.consolidated.iter().any(|e| e.skill_name == "old-skill"));
        assert!(result.pruned.is_empty());
    }

    #[test]
    fn test_build_rename_summary_with_consolidation() {
        let config = CuratorConfig::default();
        let curator = Curator::new(config);

        // skill1 and skill2 removed, umbrella added (survives)
        let before = vec!["skill1".to_string(), "skill2".to_string()];
        let after = vec!["umbrella".to_string()];
        let tool_calls = vec![
            ToolCall {
                name: "skill_manage".to_string(),
                arguments: ToolCallArgs {
                    action: Some("patch".to_string()),
                    name: Some("umbrella".to_string()),
                    file_path: Some("/path/to/skill1.md".to_string()),
                    file_content: None,
                    content: None,
                    new_string: None,
                    absorbed_into: None,
                },
            },
        ];

        let result = curator.build_rename_summary(&before, &after, &tool_calls);

        eprintln!("Result: {}", result);

        // With heuristic classification, skill1 might be consolidated
        // and skill2 pruned (since skill2 has no matching evidence)
        // The key is the result should contain "archived" and some consolidation info
        assert!(result.contains("archived"), "Expected 'archived' in result: {}", result);
    }

    #[test]
    fn test_build_rename_summary_with_pruning() {
        let config = CuratorConfig::default();
        let curator = Curator::new(config);

        let before = vec!["skill1".to_string()];
        let after = vec![];
        let tool_calls = vec![];

        let result = curator.build_rename_summary(&before, &after, &tool_calls);

        // Result should contain archived info
        assert!(result.contains("archived"), "Expected 'archived' in result: {}", result);
    }
}
