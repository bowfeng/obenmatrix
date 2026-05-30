/// Curator — orchestrator for background skill maintenance.

use crate::lifecycle::{LifecycleManager, LifecycleConfig, LifecycleState};
use crate::usage::mark_agent_created;
use tracing::{info, debug};
use std::path::PathBuf;
use chrono::{DateTime, Utc};

/// Configuration for the curator.
#[derive(Debug, Clone)]
pub struct CuratorConfig {
    /// Interval between curator runs (in hours).
    pub interval_hours: usize,
    /// Minimum idle time before running (in hours).
    pub min_idle_hours: usize,
    /// Directory containing skills.
    pub skills_dir: PathBuf,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            interval_hours: 24 * 7, // 7 days
            min_idle_hours: 2,
            skills_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".obenalien")
                .join("skills"),
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
            None => true, // Never run before
            Some(last_run) => {
                let hours_since = (Utc::now() - last_run).num_hours();
                hours_since >= self.interval_hours as i64
            }
        }
    }

    /// Record a run completion.
    pub fn record_run(&mut self, summary: String, duration_seconds: f64) {
        self.last_run_at = Some(Utc::now());
        self.last_run_duration_seconds = Some(duration_seconds);
        self.last_run_summary = Some(summary);
        self.run_count += 1;
    }
}

/// Curator — orchestrates skill maintenance tasks.
pub struct Curator {
    #[allow(dead_code)]
    config: CuratorConfig,
    lifecycle_manager: LifecycleManager,
    state: CuratorState,
}

impl Curator {
    pub fn new(config: CuratorConfig) -> Self {
        let lifecycle_config = LifecycleConfig {
            stale_after_days: 30,
            archive_after_days: 90,
        };
        let mut state = CuratorState::new();
        state.interval_hours = config.interval_hours;
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
        let stale_count = changes.iter().filter(|(_, s)| *s == LifecycleState::Stale).count();
        let archived_count = changes.iter().filter(|(_, s)| *s == LifecycleState::Archived).count();

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
    }

    #[test]
    fn test_curator_state_should_run_never_run() {
        let state = CuratorState::new();
        assert!(state.should_run(200.0)); // Should run if never run (idle > 168h default)
    }

    #[test]
    fn test_curator_state_should_run_paused() {
        let mut state = CuratorState::new();
        state.paused = true;
        assert!(!state.should_run(100.0)); // Should not run if paused
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
        assert!(!curator.is_due(1.0)); // Not due on first run check
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
}
