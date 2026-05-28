/// Cronjob tool — cron job management via a single compressed action-oriented tool.
///
/// Supports create, list, update, pause, resume, remove, and trigger actions.

use std::sync::LazyLock;
use std::sync::Mutex;
use serde_json::Value;
use chrono::Utc;

use oben_cron::{CronJob, CronStore, CronUpdate, scan_cron_prompt, DeliverTarget, JobState};
use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_deliver(deliver_str: &str) -> Result<DeliverTarget, String> {
    let s = deliver_str.trim();
    Ok(match s {
        "local" => DeliverTarget::Local,
        "origin" => DeliverTarget::Origin,
        "all" => DeliverTarget::All,
        _ => {
            if let Some(idx) = s.find(':') {
                let platform = &s[..idx];
                let ref_str = &s[idx + 1..];
                if platform.is_empty() || ref_str.is_empty() {
                    return Err(format!(
                        "Invalid deliver format '{}'. Use 'local', 'origin', 'all', or 'platform:ref'",
                        deliver_str
                    ));
                }
                DeliverTarget::Platform(ref_str.to_string())
            } else {
                return Err(format!(
                    "Invalid deliver target '{}'. Use 'local', 'origin', 'all', or 'platform:ref'",
                    deliver_str
                ));
            }
        }
    })
}

fn format_datetime(dt: &Option<chrono::DateTime<chrono::Utc>>) -> String {
    match dt {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => "N/A".to_string(),
    }
}

fn format_status(job: &CronJob) -> String {
    let enabled = if job.enabled { "enabled" } else { "disabled" };
    let state = match job.state {
        JobState::Scheduled => "scheduled",
        JobState::Paused => "paused",
        JobState::Completed => "completed",
        JobState::Error => "error",
    };
    format!("{}, {}", enabled, state)
}

/// Open or create a CronStore at the default config path.
fn open_store() -> Result<CronStore, String> {
    CronStore::new(CronStore::default_path()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_cronjob_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "action".into(),
            description: "Action: create, list, update, pause, resume, remove, trigger.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "job_id".into(),
            description: "Job ID or name (for update/pause/resume/remove/trigger).".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "prompt".into(),
            description: "Prompt text (for create/update).".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "schedule".into(),
            description: "Schedule: '30m', 'every 2h', '0 9 * * *', ISO timestamp.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "name".into(),
            description: "Human-friendly job name.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "repeat".into(),
            description: "Repeat count (0=infinite).".into(),
            parameter_type: "u32".into(),
            required: false,
        },
        ToolParameter {
            name: "deliver".into(),
            description: "Delivery target: 'local', 'origin', 'all', or 'platform:ref'.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "skills".into(),
            description: "Comma-separated skill names.".into(),
            parameter_type: "string".into(),
            required: false,
        },
        ToolParameter {
            name: "include_disabled".into(),
            description: "Include disabled jobs in list.".into(),
            parameter_type: "bool".into(),
            required: false,
        },
    ];
    Tool {
        name: "cronjob".into(),
        description: "Manage cron jobs. Actions: create, list, update, pause, resume, remove, trigger.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_cronjob_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let call_id = args.get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let action = args.get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' argument"))?;

            let store = open_store()?;

            match action {
                "create" => action_create(&store, &call_id, &args),
                "list" => action_list(&store, &call_id, &args),
                "update" => action_update(&store, &call_id, &args),
                "pause" => action_pause(&store, &call_id, &args),
                "resume" => action_resume(&store, &call_id, &args),
                "remove" => action_remove(&store, &call_id, &args),
                "trigger" => action_trigger(&store, &call_id, &args),
                _ => Ok(ToolResult {
                    call_id,
                    output: String::new(),
                    error: Some(format!(
                        "Unknown action '{}'. Use: create, list, update, pause, resume, remove, trigger.",
                        action
                    )),
                }),
            }
        })
    })
}

fn action_create(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let prompt = args.get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'prompt' is required for 'create'"))?;

    // Scan prompt for security
    scan_cron_prompt(prompt).map_err(|e| anyhow::anyhow!("{}", e))?;

    let schedule = args.get("schedule")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'schedule' is required for 'create'"))?;

    let name = args.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unnamed Job")
        .to_string();

    let repeat = args.get("repeat")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let deliver = match args.get("deliver").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => parse_deliver(s)?,
        _ => DeliverTarget::default(),
    };

    let skills = match args.get("skills").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.split(',')
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .collect(),
        _ => Vec::new(),
    };

    let mut job = CronJob::new(name, prompt.to_string(), schedule, repeat)?;
    job.deliver = deliver;
    job.skills = skills;

    store.create(job)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Created cron job: {}", prompt.chars().take(60).collect::<String>()),
        error: None,
    })
}

fn action_list(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let include_disabled = args.get("include_disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let jobs = store.list_jobs(include_disabled);

    let label = if include_disabled {
        format!("Cron Jobs ({}):", jobs.len())
    } else {
        format!("Cron Jobs ({}):", jobs.len())
    };

    let mut output = format!("{}\n", label);

    if jobs.is_empty() {
        output.push_str("\nNo cron jobs found.\n");
        return Ok(ToolResult {
            call_id: call_id.to_string(),
            output,
            error: None,
        });
    }

    for job in &jobs {
        output.push_str("\n");
        output.push_str(&format!("[{}] {} — {}\n", job.id, job.name, format_status(job)));
        let sched = job.schedule_obj.display();
        output.push_str(&format!("  Schedule: {}\n", sched));
        output.push_str(&format!("  Next run: {}\n", format_datetime(&job.next_run_at)));
        output.push_str(&format!("  Last run: {}\n", format_datetime(&job.last_run_at)));

        let prompt_display = if job.prompt.len() > 100 {
            format!("{}...", &job.prompt[..100])
        } else {
            job.prompt.clone()
        };
        output.push_str(&format!("  Prompt: {}\n", prompt_display));
    }

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output,
        error: None,
    })
}

fn action_update(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let job_id = args.get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'job_id' is required for 'update'"))?;

    // Check if a new prompt is provided and scan it
    if let Some(prompt_val) = args.get("prompt").and_then(|v| v.as_str()) {
        if !prompt_val.is_empty() {
            scan_cron_prompt(prompt_val).map_err(|e| anyhow::anyhow!("{}", e))?;
        }
    }

    let mut updates = CronUpdate::default();

    if let Some(prompt_val) = args.get("prompt").and_then(|v| v.as_str()) {
        if !prompt_val.is_empty() {
            updates.prompt = Some(prompt_val.to_string());
        }
    }

    if let Some(name_val) = args.get("name").and_then(|v| v.as_str()) {
        if !name_val.is_empty() {
            updates.name = Some(name_val.to_string());
        }
    }

    if let Some(schedule_val) = args.get("schedule").and_then(|v| v.as_str()) {
        if !schedule_val.is_empty() {
            updates.schedule = Some(schedule_val.to_string());
        }
    }

    if let Some(deliver_val) = args.get("deliver").and_then(|v| v.as_str()) {
        if !deliver_val.is_empty() {
            updates.deliver = Some(parse_deliver(deliver_val)?);
        }
    }

    if let Some(skills_val) = args.get("skills").and_then(|v| v.as_str()) {
        if !skills_val.is_empty() {
            updates.skills = Some(skills_val.split(',')
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty())
                .collect());
        }
    }

    if let Some(repeat_val) = args.get("repeat").and_then(|v| v.as_u64()) {
        updates.repeat = Some(repeat_val as u32);
    }

    let _updated = store.update_job(job_id, updates)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Updated job: {}", job_id),
        error: None,
    })
}

fn action_pause(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let job_id = args.get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'job_id' is required for 'pause'"))?;

    store.pause(job_id)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Paused job: {}", job_id),
        error: None,
    })
}

fn action_resume(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let job_id = args.get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'job_id' is required for 'resume'"))?;

    store.resume(job_id)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Resumed job: {}", job_id),
        error: None,
    })
}

fn action_remove(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let job_id = args.get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'job_id' is required for 'remove'"))?;

    store.remove(job_id)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Removed job: {}", job_id),
        error: None,
    })
}

fn action_trigger(store: &CronStore, call_id: &str, args: &Value) -> Result<ToolResult, anyhow::Error> {
    let job_id = args.get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'job_id' is required for 'trigger'"))?;

    // Resolve ober_bin binary
    let ober_bin = resolve_ober_bin();

    store.trigger_job(job_id, &ober_bin)?;

    Ok(ToolResult {
        call_id: call_id.to_string(),
        output: format!("Triggered job: {}", job_id),
        error: None,
    })
}

/// Resolve the ober_bin binary path for cron execution.
fn resolve_ober_bin() -> String {
    if let Ok(val) = std::env::var("OBEN_BIN") {
        return val;
    }
    let pwd = std::env::current_dir().ok();
    if let Some(ref p) = pwd {
        for c in &[
            "target/debug/obenalien",
            "target/release/obenalien",
            "./target/debug/obenalien",
            "./target/release/obenalien",
        ] {
            let full_path = p.join(c);
            if full_path.exists() {
                return full_path.to_string_lossy().to_string();
            }
        }
    }
    // Check which
    if let Ok(out) = std::process::Command::new("which")
        .arg("obenalien")
        .output()
    {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct CronJobTool;

impl SelfRegisteringTool for CronJobTool {
    fn tool() -> Tool {
        make_cronjob_tool()
    }

    fn handler() -> ToolHandler {
        make_cronjob_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    CronJobTool::register_self(registry);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> super::super::registry::ToolRegistry {
        let mut registry = super::super::registry::ToolRegistry::new();
        CronJobTool::register_self(&mut registry);
        registry
    }

    #[tokio::test]
    async fn rejects_invalid_action() {
        let registry = make_registry();
        let result = registry.execute("cronjob", &json!({
            "action": "invalid",
            "call_id": "test-action",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn rejects_missing_action() {
        let registry = make_registry();
        let result = registry.execute("cronjob", &json!({
            "call_id": "test-action",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Missing 'action'"));
    }

    #[test]
    fn test_parse_deliver_local() {
        let result = parse_deliver("local");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), DeliverTarget::Local));
    }

    #[test]
    fn test_parse_deliver_origin() {
        let result = parse_deliver("origin");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), DeliverTarget::Origin));
    }

    #[test]
    fn test_parse_deliver_all() {
        let result = parse_deliver("all");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), DeliverTarget::All));
    }

    #[test]
    fn test_parse_deliver_platform_ref() {
        let result = parse_deliver("platform:telegram");
        assert!(result.is_ok());
        match result.unwrap() {
            DeliverTarget::Platform(ref_str) => assert_eq!(ref_str, "telegram"),
            _ => panic!("Expected Platform variant"),
        }
    }

    #[test]
    fn test_parse_deliver_invalid_no_colon() {
        let result = parse_deliver("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_deliver_empty_parts() {
        let result = parse_deliver("platform:");
        assert!(result.is_err());

        let result = parse_deliver("platform");
        assert!(result.is_err());
    }

    #[test]
    fn test_format_datetime_some() {
        let dt = Some(Utc::now());
        let formatted = format_datetime(&dt);
        assert!(formatted.contains("-"));
    }

    #[test]
    fn test_format_datetime_none() {
        let formatted = format_datetime(&None);
        assert_eq!(formatted, "N/A");
    }
}
