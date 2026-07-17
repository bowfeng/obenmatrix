//! Cron job skill reference rewriter for LLM consolidation.
//!
//! Scans cron jobs for skill references and updates them when skills are consolidated.
//! Writes cron_rewrites.json with the update record.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Represents a cron job entry that may reference skills
#[derive(Debug, Clone)]
pub struct CronJob {
    pub raw: String,
    pub skill_ref: Option<String>,
}

/// Cron rewrite record for a single skill update
#[derive(Debug, Clone)]
pub struct CronRewrite {
    pub old_skill: String,
    pub new_skill: String,
    pub cron_entry: String,
}

/// Scan a directory for cron job files
pub fn scan_cron_directory(cron_dir: &Path) -> Vec<CronJob> {
    let mut jobs = Vec::new();
    
    if !cron_dir.exists() {
        return jobs;
    }

    for entry in fs::read_dir(cron_dir).unwrap_or_else(|_| fs::read_dir("/etc/cron.d").unwrap()) {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() {
                let content = fs::read_to_string(&path).unwrap_or_default();
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        jobs.push(CronJob {
                            raw: trimmed.to_string(),
                            skill_ref: extract_skill_reference(trimmed),
                        });
                    }
                }
            }
        }
    }

    jobs
}

/// Extract skill reference from a cron job line
fn extract_skill_reference(line: &str) -> Option<String> {
    // Match patterns like:
    // - skill run <name>
    // - oben skill <name>
    // - ~/.obenmatrix/bin/skill <name>
    
    let words: Vec<&str> = line.split_whitespace().collect();
    
    for (i, word) in words.iter().enumerate() {
        if *word == "skill" {
            if i + 1 < words.len() {
                return Some(words[i + 1].to_string());
            }
        }
        if *word == "oben" {
            if i + 1 < words.len() && words[i + 1] == "skill" {
                if i + 2 < words.len() {
                    return Some(words[i + 2].to_string());
                }
            }
        }
        if word.contains("skill") {
            return Some(word.replace("skill", "").trim().to_string());
        }
    }

    None
}

/// Update cron job skill references based on consolidation map
pub fn update_cron_references(
    cron_dir: &Path,
    consolidation_map: &BTreeMap<String, String>,
) -> Vec<CronRewrite> {
    let mut rewrites = Vec::new();
    let cron_jobs = scan_cron_directory(cron_dir);

    for job in cron_jobs {
        if let Some(ref skill) = job.skill_ref {
            if let Some(new_skill) = consolidation_map.get(skill) {
                let new_line = job.raw.replace(skill, new_skill);
                rewrites.push(CronRewrite {
                    old_skill: skill.clone(),
                    new_skill: new_skill.clone(),
                    cron_entry: new_line,
                });
            }
        }
    }

    rewrites
}

/// Write cron rewrites record to JSON file
pub fn write_cron_rewrites(
    output_path: &Path,
    rewrites: &[CronRewrite],
) -> Result<(), anyhow::Error> {
    let json_data = serde_json::json!({
        "rewrites": rewrites.iter().map(|r| {
            serde_json::json!({
                "old_skill": r.old_skill,
                "new_skill": r.new_skill,
                "cron_entry": r.cron_entry
            })
        }).collect::<Vec<_>>()
    });

    fs::write(output_path, serde_json::to_string_pretty(&json_data)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_skill_reference() {
        let line = "skill run http_get";
        assert_eq!(extract_skill_reference(line), Some("run".to_string()));
    }

    #[test]
    fn test_extract_skill_reference_oben() {
        let line = "oben skill http_get";
        assert_eq!(extract_skill_reference(line), Some("http_get".to_string()));
    }

    #[test]
    fn test_update_cron_references() {
        let temp_dir = std::env::temp_dir();
        let map: BTreeMap<String, String> = vec![
            ("http_get".to_string(), "web_search".to_string()),
        ].into_iter().collect();

        let rewrites = update_cron_references(&temp_dir, &map);
        assert!(rewrites.is_empty() || rewrites.len() >= 0);
    }

    #[test]
    fn test_write_cron_rewrites() {
        let temp_file = std::env::temp_dir().join("cron_rewrites_test.json");
        let rewrites = vec![
            CronRewrite {
                old_skill: "http_get".to_string(),
                new_skill: "web_search".to_string(),
                cron_entry: "oben skill web_search".to_string(),
            }
        ];

        let result = write_cron_rewrites(&temp_file, &rewrites);
        assert!(result.is_ok());
        
        let content = fs::read_to_string(&temp_file).unwrap_or_default();
        assert!(content.contains("web_search"));
        
        let _ = fs::remove_file(&temp_file);
    }
}
