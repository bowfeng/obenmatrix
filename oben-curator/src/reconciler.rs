//! Classification reconciler for LLM skill consolidation.
//!
//! Merges multiple classification signals:
//! - Absorbed_into (authoritative from LLM YAML)
//! - Model YAML extraction
//! - Heuristic analysis
//!
//! Order: absorbed_into → model → heuristic

use std::collections::BTreeMap;

/// Absorption entry - a skill that was absorbed into another
#[derive(Debug, Clone, PartialEq)]
pub struct AbsorptionEntry {
    pub skill_name: String,
    pub absorbed_into: String,
    pub confidence: f32,
}

/// Classification result from reconciliation
#[derive(Debug, Clone, Default)]
pub struct ClassificationResult {
    /// Skills that were consolidated (absorbed into others)
    pub consolidated: Vec<AbsorptionEntry>,
    /// Skills that were pruned (removed entirely)
    pub pruned: Vec<String>,
    /// Source of each classification (for debugging)
    pub sources: BTreeMap<String, String>,
}

/// Reconcile classification signals from multiple sources
///
/// Priority order (highest to lowest):
/// 1. absorbed_into - authoritative from LLM YAML
/// 2. model_yml - extracted from model output
/// 3. heuristic - fallback analysis
///
/// # Arguments
/// * `absorbed_into` - Skills explicitly marked for absorption (authoritative)
/// * `model_yml` - YAML-parsed suggestions from LLM
/// * `heuristic` - Heuristic-based classifications
///
/// # Returns
/// Merged classification result with sources tracked
pub fn reconcile_classifications(
    absorbed_into: Vec<AbsorptionEntry>,
    model_yml: Option<Vec<AbsorptionEntry>>,
    heuristic: Vec<AbsorptionEntry>,
) -> ClassificationResult {
    let mut result = ClassificationResult::default();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Phase 1: Process absorbed_into (authoritative)
    for entry in absorbed_into {
        result.sources.insert(entry.skill_name.clone(), "absorbed_into".to_string());
        if seen.insert(entry.skill_name.clone()) {
            result.consolidated.push(entry);
        }
    }

    // Phase 2: Process model_yml (override if not in absorbed_into)
    if let Some(model_entries) = model_yml {
        for entry in model_entries {
            if !seen.contains(&entry.skill_name) {
                result.sources.insert(entry.skill_name.clone(), "model_yml".to_string());
                if seen.insert(entry.skill_name.clone()) {
                    result.consolidated.push(entry);
                }
            }
        }
    }

    // Phase 3: Process heuristic (fallback)
    for entry in heuristic {
        if !seen.contains(&entry.skill_name) {
            result.sources.insert(entry.skill_name.clone(), "heuristic".to_string());
            if seen.insert(entry.skill_name.clone()) {
                result.consolidated.push(entry);
            }
        }
    }

    result
}

/// Extract YAML from LLM response string
///
/// Looks for YAML frontmatter or embedded YAML blocks in the response.
/// Returns None if no valid YAML found.
pub fn extract_yaml_from_response(response: &str) -> Option<Vec<AbsorptionEntry>> {
    // Simple heuristic: look for YAML frontmatter or bullet list with skill info
    // Real implementation would use a YAML parser
    let lines: Vec<&str> = response.lines().collect();
    
    let mut entries = Vec::new();
    
    for line in lines {
        // Look for bullet list items like "- skill -> target"
        if line.trim().starts_with("- ") && line.contains("->") {
            let parts: Vec<&str> = line.split("->").collect();
            if parts.len() >= 2 {
                let skill = parts[0].trim().strip_prefix("- ").unwrap_or("").to_string();
                let target = parts[1].trim().to_string();
                
                if !skill.is_empty() && !target.is_empty() {
                    entries.push(AbsorptionEntry {
                        skill_name: skill,
                        absorbed_into: target,
                        confidence: 0.9,
                    });
                }
            }
        }
    }
    
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Heuristic-based classification of skills for consolidation
///
/// Analyzes skill names and usage patterns to suggest consolidations.
/// This is a fallback when LLM extraction fails.
pub fn heuristic_classify(skills: &[String]) -> Vec<AbsorptionEntry> {
    let mut entries = Vec::new();
    
    // Group skills by common prefixes
    let mut prefix_groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    
    for skill in skills {
        // Extract prefix (first word before underscore or hyphen)
        let prefix = skill.split(|c| c == '_' || c == '-')
            .next()
            .unwrap_or(skill.as_str())
            .to_string();
        
        prefix_groups.entry(prefix).or_default().push(skill.clone());
    }
    
    // Suggest consolidation for groups with > 1 skill
    for (prefix, group) in prefix_groups {
        if group.len() > 1 {
            // Use first skill as umbrella
            let umbrella = format!("{}_utils", prefix);
            for skill in group {
                if skill != umbrella {
                    entries.push(AbsorptionEntry {
                        skill_name: skill,
                        absorbed_into: umbrella.clone(),
                        confidence: 0.5, // Heuristic confidence
                    });
                }
            }
        }
    }
    
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BDD Test Block Example
    /// Given: Absorbed_into has a skill entry
    /// When: reconcile_classifications is called
    /// Then: Absorbed_into takes priority, result includes the entry
    #[test]
    fn test_reconciler_absorbed_into_wins() {
        let absorbed = vec![AbsorptionEntry {
            skill_name: "http_get".to_string(),
            absorbed_into: "web_search".to_string(),
            confidence: 0.95,
        }];

        let result = reconcile_classifications(absorbed.clone(), None, Vec::new());

        assert_eq!(result.consolidated.len(), 1);
        assert_eq!(result.consolidated[0].skill_name, "http_get");
        assert_eq!(result.sources.get("http_get"), Some(&"absorbed_into".to_string()));
    }

    /// BDD Test Block Example
    /// Given: Model YAML and heuristic both have entries
    /// When: reconcile_classifications is called
    /// Then: Model YAML processed before heuristic
    #[test]
    fn test_reconciler_model_yml_before_heuristic() {
        let model_entries = vec![AbsorptionEntry {
            skill_name: "model_skill".to_string(),
            absorbed_into: "umbrella".to_string(),
            confidence: 0.8,
        }];

        let heuristic_entries = vec![AbsorptionEntry {
            skill_name: "model_skill".to_string(),
            absorbed_into: "different_umbrella".to_string(),
            confidence: 0.5,
        }];

        let result = reconcile_classifications(Vec::new(), Some(model_entries), heuristic_entries);

        // Model entry should be in result, not heuristic override
        assert_eq!(result.consolidated.len(), 1);
        assert_eq!(result.consolidated[0].absorbed_into, "umbrella");
    }

    /// BDD Test Block Example
    /// Given: LLM response with YAML frontmatter
    /// When: extract_yaml_from_response is called
    /// Then: Returns parsed absorption entries
    #[test]
    fn test_extract_yaml_from_response() {
        let response = r#"
Here are the consolidation suggestions:

- http_get -> web_search
- http_post -> web_search
"#;

        let result = extract_yaml_from_response(response);

        assert!(result.is_some());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].skill_name, "http_get");
        assert_eq!(entries[0].absorbed_into, "web_search");
    }

    /// BDD Test Block Example
    /// Given: No YAML in LLM response
    /// When: extract_yaml_from_response is called
    /// Then: Returns None
    #[test]
    fn test_extract_yaml_from_response_none() {
        let response = "No yaml here, just text";

        let result = extract_yaml_from_response(response);

        assert!(result.is_none());
    }

    /// BDD Test Block Example
    /// Given: Skills with common prefixes
    /// When: heuristic_classify is called
    /// Then: Returns consolidation suggestions for similar skills
    #[test]
    fn test_heuristic_classify() {
        let skills = vec![
            "http_get".to_string(),
            "http_post".to_string(),
            "http_delete".to_string(),
            "file_write".to_string(),
        ];

        let result = heuristic_classify(&skills);

        // Should group http_* skills together
        assert!(result.iter().any(|e| e.skill_name == "http_get" || e.skill_name == "http_post"));
    }
}
