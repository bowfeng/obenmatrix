/// Curator reporting — generate human-readable summaries of skill maintenance.
use crate::reconciler::ClassificationResult;
use crate::usage::load_usage;

use chrono::Utc;
use std::fs;
use std::path::Path;
use tracing::info;

/// Generate a comprehensive report of all skills and their maintenance status.
pub fn generate_report() -> String {
    let records = load_usage();

    if records.is_empty() {
        return "No skills tracked yet.".to_string();
    }

    let mut report = String::from("## Curator Report\n\n");

    // Summary statistics
    let total = records.len();
    let agent_created = records
        .values()
        .filter(|r| r.created_by.as_deref() == Some("agent"))
        .count();
    let total_uses: usize = records.values().map(|r| r.use_count).sum();
    let total_views: usize = records.values().map(|r| r.view_count).sum();
    let total_patches: usize = records.values().map(|r| r.patch_count).sum();

    report.push_str(&format!(
        "Total skills: {}\nAgent-created: {}\n\n",
        total, agent_created
    ));

    report.push_str(&format!(
        "Total uses: {} | Views: {} | Patches: {}\n\n",
        total_uses, total_views, total_patches
    ));

    // Skills by category
    report.push_str("### Skills by Activity\n\n");

    // Sort by use_count (descending)
    let mut sorted: Vec<_> = records
        .iter()
        .map(|(name, record)| (name, record))
        .collect();
    sorted.sort_by(|(_, a), (_, b)| b.use_count.cmp(&a.use_count));

    report.push_str("| Skill | Uses | Views | Patches | State |\n");
    report.push_str("|-------|------|-------|---------|-------|\n");

    for (name, record) in &sorted {
        let state = match &record.state {
            Some(s) if s == "archived" => "📦 Archived",
            Some(s) if s == "stale" => "⏸️ Stale",
            Some(_) => "✅ Active",
            None => "✅ Active",
        };
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            name, record.use_count, record.view_count, record.patch_count, state
        ));
    }

    // Pinned skills
    let pinned: Vec<_> = records
        .iter()
        .filter(|(_, r)| r.pinned)
        .map(|(name, _)| name)
        .collect();

    if !pinned.is_empty() {
        report.push_str("\n### Pinned Skills\n\n");
        for name in &pinned {
            report.push_str(&format!("- {}\n", name));
        }
    }

    // Low activity skills (potential candidates for archival)
    let low_activity: Vec<_> = records
        .iter()
        .filter(|(_, r)| r.use_count < 3 && !r.pinned)
        .map(|(name, _)| name)
        .collect();

    if !low_activity.is_empty() {
        report.push_str("\n### Low Activity Skills (review recommended)\n\n");
        for name in &low_activity {
            report.push_str(&format!("- {}\n", name));
        }
    }

    info!("Generated curator report with {} skills", total);
    report
}

/// Generate a compact summary for CLI display.
pub fn generate_summary() -> String {
    let records = load_usage();

    let total = records.len();
    let agent_created = records
        .values()
        .filter(|r| r.created_by.as_deref() == Some("agent"))
        .count();

    format!(
        "Curator: {} skills tracked ({} agent-created)",
        total, agent_created
    )
}

/// Generate a JSON report for machine consumption.
pub fn generate_json_report() -> String {
    let records = load_usage();

    let mut json_data = serde_json::Map::new();

    // Summary stats
    let total: usize = records.len();
    let agent_created: usize = records
        .values()
        .filter(|r| r.created_by.as_deref() == Some("agent"))
        .count();

    json_data.insert(
        "total_skills".to_string(),
        serde_json::Value::Number(serde_json::Number::from(total)),
    );
    json_data.insert(
        "agent_created".to_string(),
        serde_json::Value::Number(serde_json::Number::from(agent_created)),
    );

    // Skills array
    let mut skills = Vec::new();
    for (name, record) in &records {
        let mut skill_data = serde_json::Map::new();
        skill_data.insert("name".to_string(), serde_json::Value::String(name.clone()));
        skill_data.insert(
            "use_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.use_count)),
        );
        skill_data.insert(
            "view_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.view_count)),
        );
        skill_data.insert(
            "patch_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.patch_count)),
        );
        skill_data.insert(
            "state".to_string(),
            serde_json::Value::String(record.state.clone().unwrap_or_else(|| "active".to_string())),
        );
        skill_data.insert("pinned".to_string(), serde_json::Value::Bool(record.pinned));
        skill_data.insert(
            "created_by".to_string(),
            serde_json::Value::String(
                record
                    .created_by
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
        );
        skills.push(serde_json::Value::Object(skill_data));
    }

    json_data.insert("skills".to_string(), serde_json::Value::Array(skills));

    serde_json::to_string_pretty(&json_data).unwrap_or_else(|_| "{}".to_string())
}

fn parse_skill_arrow(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    
    if let Some(idx) = line.find(" -> ") {
        let parts: Vec<&str> = line.splitn(2, " -> ").collect();
        if parts.len() == 2 {
            return Some((parts[0].trim().to_string(), parts[1].trim().to_string()));
        }
    }
    
    if let Some(idx) = line.find(" → ") {
        let parts: Vec<&str> = line.splitn(2, " → ").collect();
        if parts.len() == 2 {
            return Some((parts[0].trim().to_string(), parts[1].trim().to_string()));
        }
    }
    
    None
}

pub struct ConsolidationReport {
    pub consolidated: Vec<String>,
    pub pruned: Vec<String>,
    pub dry_run: bool,
    pub timestamp: String,
}

impl ConsolidationReport {
    pub fn new() -> Self {
        Self {
            consolidated: Vec::new(),
            pruned: Vec::new(),
            dry_run: false,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    pub fn skipped() -> Self {
        Self {
            consolidated: Vec::new(),
            pruned: Vec::new(),
            dry_run: true,
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

pub fn generate_consolidation_reports(
    report_dir: &Path,
    consolidation: &ClassificationResult,
    cron_rewrites: &[String],
    dry_run: bool,
) -> Result<(), anyhow::Error> {
    fs::create_dir_all(report_dir)?;

    let run_json_path = report_dir.join("run.json");
    let run_json = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "dry_run": dry_run,
        "consolidated": consolidation.consolidated.iter().map(|e| e.skill_name.clone()).collect::<Vec<_>>(),
        "pruned": consolidation.pruned.clone(),
        "sources": consolidation.sources.clone(),
    });
    fs::write(&run_json_path, serde_json::to_string_pretty(&run_json)?)?;

    let report_md_path = report_dir.join("REPORT.md");
    let mut report_md = String::from("# Curator Consolidation Report\n\n");
    report_md.push_str(&format!("Timestamp: {}\n\n", Utc::now().to_rfc3339()));
    report_md.push_str(&format!("Dry Run: {}\n\n", dry_run));
    
    report_md.push_str("## Skills Consolidated\n\n");
    if consolidation.consolidated.is_empty() {
        report_md.push_str("*No skills consolidated*\n\n");
    } else {
        for entry in &consolidation.consolidated {
            report_md.push_str(&format!(
                "- **{}** → **{}** (confidence: {:.0}%, source: {})\n",
                entry.skill_name, entry.absorbed_into, entry.confidence * 100.0,
                consolidation.sources.get(&entry.skill_name).unwrap_or(&"unknown".to_string())
            ));
        }
        report_md.push('\n');
    }

    report_md.push_str("## Skills Pruned\n\n");
    if consolidation.pruned.is_empty() {
        report_md.push_str("*No skills pruned*\n\n");
    } else {
        for skill in &consolidation.pruned {
            report_md.push_str(&format!("- **{}**\n", skill));
        }
        report_md.push('\n');
    }

    report_md.push_str("## Cron Job Rewrites\n\n");
    if cron_rewrites.is_empty() {
        report_md.push_str("*No cron job updates needed*\n");
    } else {
        for rewrite in cron_rewrites {
            report_md.push_str(&format!("- {}\n", rewrite));
        }
    }

    fs::write(&report_md_path, report_md)?;

    let cron_json_path = report_dir.join("cron_rewrites.json");
    fs::write(&cron_json_path, serde_json::to_string_pretty(&cron_rewrites)?)?;

    info!("Consolidation reports written to {}", report_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AbsorptionEntry;
    use std::path::PathBuf;

    #[test]
    fn test_generate_summary_no_records() {
        let summary = generate_summary();
        assert!(summary.contains("Curator:"));
    }

    #[test]
    fn test_generate_report_no_records() {
        let report = generate_report();
        assert!(report.contains("No skills tracked yet."));
    }

    #[test]
    fn test_generate_json_report_no_records() {
        let json = generate_json_report();
        assert!(json.contains("\"total_skills\""));
        assert!(json.contains("0"));
    }

    #[test]
    fn test_consolidation_report_new() {
        let report = ConsolidationReport::new();
        assert!(!report.timestamp.is_empty());
        assert!(report.consolidated.is_empty());
        assert!(report.pruned.is_empty());
    }

    #[test]
    fn test_consolidation_report_skipped() {
        let report = ConsolidationReport::skipped();
        assert!(report.dry_run);
    }

    #[test]
    fn test_generate_consolidation_reports() {
        let temp_dir = std::env::temp_dir().join(format!("consolidation-test-{:?}", Utc::now().timestamp()));
        let classification = ClassificationResult {
            consolidated: vec![
                AbsorptionEntry {
                    skill_name: "http_get".to_string(),
                    absorbed_into: "web_search".to_string(),
                    confidence: 0.95,
                }
            ],
            pruned: vec!["old_skill".to_string()],
            sources: vec![("http_get".to_string(), "absorbed_into".to_string())].into_iter().collect(),
        };

        let result = generate_consolidation_reports(
            &temp_dir,
            &classification,
            &["curl -> web_search".to_string()],
            false,
        );

        assert!(result.is_ok());
        assert!(temp_dir.join("run.json").exists());
        assert!(temp_dir.join("REPORT.md").exists());
        assert!(temp_dir.join("cron_rewrites.json").exists());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
