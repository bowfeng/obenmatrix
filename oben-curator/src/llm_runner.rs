//! LLM runner for curator consolidation pass.
//!
//! Extracts consolidation suggestions from LLM responses.

/// Extract consolidation suggestions from LLM response
/// Looks for arrow format like "skill1 -> skill2" or "skill1 → skill2"
pub fn extract_yaml_from_consolidation_response(response: &str) -> Option<Vec<(String, String)>> {
    let mut suggestions = Vec::new();
    for line in response.lines() {
        if let Some((skill, target)) = parse_skill_arrow_internal(line) {
            suggestions.push((skill, target));
        }
    }
    
    if suggestions.is_empty() {
        None
    } else {
        Some(suggestions)
    }
}

fn parse_skill_arrow_internal(line: &str) -> Option<(String, String)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_yaml_from_consolidation_response() {
        let response = "skill1 -> skill2\nskill3 -> skill4";
        let result = extract_yaml_from_consolidation_response(response);
        assert!(result.is_some());
        let suggestions = result.unwrap();
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0], ("skill1".to_string(), "skill2".to_string()));
    }

    #[test]
    fn test_extract_yaml_from_consolidation_response_unicode() {
        let response = "skill1 → skill2";
        let result = extract_yaml_from_consolidation_response(response);
        assert!(result.is_some());
        let suggestions = result.unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0], ("skill1".to_string(), "skill2".to_string()));
    }

    #[test]
    fn test_extract_yaml_from_consolidation_response_no_match() {
        let response = "No skill arrows here";
        let result = extract_yaml_from_consolidation_response(response);
        assert!(result.is_none());
    }
}
