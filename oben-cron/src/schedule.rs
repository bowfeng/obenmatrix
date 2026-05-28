//! Schedule types for cron jobs.
//!
//! Supports 4 schedule types:
//! - Once (ISO timestamp or duration from now) — run once at a specific time
//! - Interval — run every N minutes
//! - Cron — standard 5-field crontab expression

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Schedule representation parsed from a string or configured directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum Schedule {
    /// Run once at a specific time.
    Once {
        run_at: String,
        display: String,
    },
    /// Repeat every N minutes.
    Interval {
        #[serde(rename = "minutes")]
        interval_minutes: u64,
        display: String,
    },
    /// Standard 5-field crontab expression.
    Cron {
        expr: String,
        display: String,
    },
}

impl Schedule {
    /// Returns the interval in minutes if this is an Interval schedule.
    pub fn interval_minutes(&self) -> u64 {
        match self {
            Self::Interval { interval_minutes, .. } => *interval_minutes,
            _ => 0,
        }
    }

    /// Returns true if this is an Once schedule.
    pub fn is_once(&self) -> bool {
        matches!(self, Self::Once { .. })
    }

    /// Get the next run time for this schedule, starting after `after`.
    pub fn next_run(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>> {
        match self {
            Self::Once { run_at, .. } => {
                let t = chrono::DateTime::parse_from_rfc3339(run_at)
                    .with_context(|| format!("Invalid ISO timestamp: {}", run_at))?
                    .with_timezone(&Utc);
                if t <= after {
                    anyhow::bail!("Once schedule '{}' is in the past", run_at);
                }
                Ok(t)
            }
            Self::Interval { interval_minutes, .. } => {
                let mins = *interval_minutes as i64;
                let midnight = after.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
                let elapsed = after.signed_duration_since(midnight).num_minutes();
                let next = ((elapsed / mins) + 1) * mins;
                Ok(midnight + chrono::Duration::minutes(next))
            }
            Self::Cron { ref expr, .. } => {
                let cron_expr = if expr.split_whitespace().count() == 5 {
                    format!("0 {}", expr)
                } else {
                    expr.clone()
                };
                let sched = cron::Schedule::from_str(&cron_expr)
                    .with_context(|| format!("Invalid cron expression: {}", expr))?;
                sched.upcoming(Utc).next()
                    .context("No future trigger for cron expression")
                    .map(|t| t.naive_utc().and_utc())
            }
        }
    }

    pub fn display(&self) -> &str {
        match self {
            Self::Once { display, .. } => display,
            Self::Interval { display, .. } => display,
            Self::Cron { display, .. } => display,
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::Interval { interval_minutes: 60, display: "every 60m".to_string() }
    }
}

impl std::fmt::Display for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Once { run_at, .. } => write!(f, "once at {}", run_at),
            Self::Interval { interval_minutes, .. } => write!(f, "every {}m", interval_minutes),
            Self::Cron { expr, .. } => write!(f, "cron: {}", expr),
        }
    }
}

/// Parse a schedule string into a Schedule.
///
/// Formats:
/// - Duration: "30m", "2h", "1d" -> Once (from now)
/// - ISO 8601: "2026-06-01T09:30:00Z" -> Once
/// - Interval: "every 30m", "every 2h" -> Interval
/// - Cron: "0 9 * * *" -> Cron
pub fn parse_schedule(input: &str) -> Result<Schedule> {
    let trimmed = input.trim();

    // ISO timestamp?
    if chrono::DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Ok(Schedule::Once {
            run_at: trimmed.to_string(),
            display: format!("once at {}", trimmed),
        });
    }

    // Interval: "every N<m/h>"
    if let Some(rest) = trimmed.strip_prefix("every ") {
        return parse_interval(rest);
    }

    // Duration: "30m", "2h", "1d"
    if trimmed.ends_with('m') && trimmed[..trimmed.len()-1].chars().all(|c| c.is_ascii_digit()) {
        let mins = trimmed[..trimmed.len()-1].parse::<u64>()?;
        return Ok(Schedule::Once {
            run_at: (Utc::now() + chrono::Duration::minutes(mins as i64)).to_rfc3339(),
            display: format!("{}m from now", mins),
        });
    }
    if trimmed.ends_with('h') && trimmed[..trimmed.len()-1].chars().all(|c| c.is_ascii_digit()) {
        let hours = trimmed[..trimmed.len()-1].parse::<u64>()?;
        return Ok(Schedule::Once {
            run_at: (Utc::now() + chrono::Duration::hours(hours as i64)).to_rfc3339(),
            display: format!("{}h from now", hours),
        });
    }
    if trimmed.ends_with('d') && trimmed[..trimmed.len()-1].chars().all(|c| c.is_ascii_digit()) {
        let days = trimmed[..trimmed.len()-1].parse::<u64>()?;
        return Ok(Schedule::Once {
            run_at: (Utc::now() + chrono::Duration::days(days as i64)).to_rfc3339(),
            display: format!("{}d from now", days),
        });
    }

    // Cron expression — cron crate uses 6-field format "sec min hour day month"
    // Prepend "0 " for seconds if input looks like 5-field cron (no leading digit with space, or 5 parts)
    let cron_input = if trimmed.split_whitespace().count() == 5 {
        format!("0 {}", trimmed)
    } else {
        trimmed.to_string()
    };
    if cron::Schedule::from_str(&cron_input).is_ok() {
        return Ok(Schedule::Cron {
            expr: trimmed.to_string(),  // Store user's 5-field expression as-is
            display: trimmed.to_string(),
        });
    }

    anyhow::bail!(
        "Cannot parse schedule: '{}'. Use '30m', '2h', 'every 30m', '0 9 * * *', or ISO timestamp",
        input
    )
}

fn parse_interval(input: &str) -> Result<Schedule> {
    let s = input.trim();
    if s.ends_with('m') {
        let mins = s[..s.len()-1].parse::<u64>()
            .with_context(|| format!("Invalid interval minutes: {}", s))?;
        return Ok(Schedule::Interval {
            interval_minutes: mins,
            display: format!("every {}m", mins),
        });
    }
    if s.ends_with('h') {
        let hours = s[..s.len()-1].parse::<u64>()
            .with_context(|| format!("Invalid interval hours: {}", s))?;
        return Ok(Schedule::Interval {
            interval_minutes: hours * 60,
            display: format!("every {}h", hours),
        });
    }
    anyhow::bail!("Invalid interval format: '{}'. Use '30m' or '2h'", input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interval() {
        let sched = parse_schedule("every 30m").unwrap();
        assert_eq!(sched.interval_minutes(), 30);
    }

    #[test]
    fn test_parse_cron_5field() {
        let sched = parse_schedule("0 9 * * *").unwrap();
        match &sched {
            Schedule::Cron { expr, .. } => assert_eq!(expr, "0 9 * * *"),
            _ => panic!("Expected Cron"),
        }
    }

    #[test]
    fn test_parse_cron_all() {
        let sched = parse_schedule("* * * * *").unwrap();
        match &sched {
            Schedule::Cron { .. } => {}
            _ => panic!("Expected Cron"),
        }
    }

    #[test]
    fn test_parse_iso() {
        let sched = parse_schedule("2026-06-01T09:30:00Z").unwrap();
        match &sched {
            Schedule::Once { run_at, .. } => assert_eq!(run_at, "2026-06-01T09:30:00Z"),
            _ => panic!("Expected Once"),
        }
    }

    #[test]
    fn test_parse_duration() {
        let sched = parse_schedule("30m").unwrap();
        assert!(sched.is_once());
    }

    #[test]
    fn test_next_run_interval() {
        let now = Utc::now();
        let sched = parse_schedule("every 30m").unwrap();
        let next = sched.next_run(now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_next_run_cron() {
        let now = Utc::now();
        let sched = parse_schedule("0 9 * * *").unwrap();
        let next = sched.next_run(now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_display_interval() {
        let sched = Schedule::default();
        assert!(format!("{}", sched).contains("60m"));
    }

    #[test]
    fn test_display_cron() {
        let sched = parse_schedule("0 9 * * *").unwrap();
        assert!(format!("{}", sched).contains("0 9 * * *"));
    }
}
