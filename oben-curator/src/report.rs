/// Curator reporting — generate human-readable summaries of skill maintenance.

use crate::usage::load_usage;

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
    let agent_created = records.values().filter(|r| r.created_by.as_deref() == Some("agent")).count();
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
    let mut sorted: Vec<_> = records.iter()
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
            name,
            record.use_count,
            record.view_count,
            record.patch_count,
            state
        ));
    }

    // Pinned skills
    let pinned: Vec<_> = records.iter()
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
    let low_activity: Vec<_> = records.iter()
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
    let agent_created = records.values().filter(|r| r.created_by.as_deref() == Some("agent")).count();
    
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
    let agent_created: usize = records.values().filter(|r| r.created_by.as_deref() == Some("agent")).count();
    
    json_data.insert(
        "total_skills".to_string(),
        serde_json::Value::Number(serde_json::Number::from(total))
    );
    json_data.insert(
        "agent_created".to_string(),
        serde_json::Value::Number(serde_json::Number::from(agent_created))
    );

    // Skills array
    let mut skills = Vec::new();
    for (name, record) in &records {
        let mut skill_data = serde_json::Map::new();
        skill_data.insert(
            "name".to_string(),
            serde_json::Value::String(name.clone())
        );
        skill_data.insert(
            "use_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.use_count))
        );
        skill_data.insert(
            "view_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.view_count))
        );
        skill_data.insert(
            "patch_count".to_string(),
            serde_json::Value::Number(serde_json::Number::from(record.patch_count))
        );
        skill_data.insert(
            "state".to_string(),
            serde_json::Value::String(record.state.clone().unwrap_or_else(|| "active".to_string()))
        );
        skill_data.insert(
            "pinned".to_string(),
            serde_json::Value::Bool(record.pinned)
        );
        skill_data.insert(
            "created_by".to_string(),
            serde_json::Value::String(record.created_by.clone().unwrap_or_else(|| "unknown".to_string()))
        );
        skills.push(serde_json::Value::Object(skill_data));
    }
    
    json_data.insert(
        "skills".to_string(),
        serde_json::Value::Array(skills)
    );

    serde_json::to_string_pretty(&json_data).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
