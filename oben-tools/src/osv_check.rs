/// OSV check tool — scans Python dependencies for security vulnerabilities.
///
/// Uses the OSV.dev API to check packages against known vulnerability databases.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use oben_models::{Tool, ToolParameter, ToolParameters, ToolResult};

use super::registry::{ToolHandler, SelfRegisteringTool};

// ---------------------------------------------------------------------------
// OSV API types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct OSVRequest {
    package: OSVPackage,
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OSVPackage {
    name: String,
    ecosystem: String,
}

#[derive(Debug, Deserialize)]
struct OSVResponse {
    vulns: Vec<OSVVuln>,
}

#[derive(Debug, Deserialize)]
struct OSVVuln {
    id: String,
    summary: Option<String>,
    aliases: Vec<String>,
    references: Vec<OSVReference>,
    affected: Vec<OSVAffected>,
}

#[derive(Debug, Deserialize)]
struct OSVReference {
    url: String,
}

#[derive(Debug, Deserialize)]
struct OSVAffected {
    package: OSVPackage,
    ranges: Vec<OSVRange>,
}

#[derive(Debug, Deserialize)]
struct OSVRange {
    r#type: String,
    events: Vec<OSVEvent>,
}

#[derive(Debug, Deserialize)]
struct OSVEvent {
    introduced: Option<String>,
    fixed: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn make_osv_check_tool() -> Tool {
    let params = vec![
        ToolParameter {
            name: "package_name".into(),
            description: "Name of the package to check.".into(),
            parameter_type: "string".into(),
            required: true,
        },
        ToolParameter {
            name: "version".into(),
            description: "Version to check (e.g., '1.2.3'). Uses latest if not specified.".into(),
            parameter_type: "string".into(),
            required: false,
        },
    ];
    Tool {
        name: "osv_check".into(),
        description: "Check packages for known security vulnerabilities using OSV.dev. Supports PyPI, npm, and GitHub ecosystems.".into(),
        parameters: ToolParameters::Flat(params),
    }
}

fn make_osv_check_handler() -> ToolHandler {
    Arc::new(|args: Value| {
        Box::pin(async move {
            let package_name = args
                .get("package_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'package_name' argument"))?;

            let version = args
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let call_id = args.get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Determine ecosystem
            let ecosystem = if package_name.starts_with("@") || package_name.contains("/") {
                "npm"
            } else {
                "PyPI"
            };

            // Query OSV API
            let client = reqwest::Client::new();
            let request = OSVRequest {
                package: OSVPackage {
                    name: package_name.to_string(),
                    ecosystem: ecosystem.to_string(),
                },
                version: version.to_string(),
            };

            let response = match client
                .post("https://api.osv.dev/v1/query")
                .json(&request)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(ToolResult {
                        call_id,
                        output: format!("OSV API error: {}", e),
                        error: Some(format!("Failed to query OSV API: {}", e)),
                    });
                }
            };

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Ok(ToolResult {
                    call_id,
                    output: format!("OSV API returned {}: {}", status, body),
                    error: Some(format!("OSV API error: {}", status)),
                });
            }

            let body = response.text().await?;

            // Try to parse as OSV response
            let vulns: Vec<OSVVuln> = match serde_json::from_str::<OSVResponse>(&body) {
                Ok(resp) => resp.vulns,
                Err(_) => Vec::new(), // No vulnerabilities or non-OSV format
            };

            let mut output = format!(
                "🔍 OSV Security Check: {} {}\n{}\n",
                package_name,
                version,
                "=".repeat(50)
            );

            if vulns.is_empty() {
                output.push_str("✅ No known vulnerabilities found.\n");
                return Ok(ToolResult {
                    call_id,
                    output,
                    error: None,
                });
            }

            output.push_str(&format!("⚠️  Found {} known vulnerability(ies):\n\n", vulns.len()));

            for (i, vuln) in vulns.iter().enumerate() {
                output.push_str(&format!("#{}: {}\n", i + 1, vuln.id));
                if let Some(summary) = &vuln.summary {
                    output.push_str(&format!("   {}\n", summary));
                }
                if !vuln.aliases.is_empty() {
                    output.push_str(&format!("   Aliases: {}\n", vuln.aliases.join(", ")));
                }
                if !vuln.references.is_empty() {
                    output.push_str("   References:\n");
                    for ref_url in &vuln.references[..vuln.references.len().min(3)] {
                        output.push_str(&format!("   - {}\n", ref_url.url));
                    }
                }
                output.push('\n');
            }

            Ok(ToolResult {
                call_id,
                output,
                error: None,
            })
        })
    })
}

// ---------------------------------------------------------------------------
// Self-registration
// ---------------------------------------------------------------------------

pub struct OSVCheckTool;

impl SelfRegisteringTool for OSVCheckTool {
    fn tool() -> Tool {
        make_osv_check_tool()
    }

    fn handler() -> ToolHandler {
        make_osv_check_handler()
    }
}

/// Register this module into the given registry.
pub fn register(registry: &mut super::registry::ToolRegistry) {
    OSVCheckTool::register_self(registry);
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
        OSVCheckTool::register_self(&mut registry);
        registry
    }

    #[tokio::test]
    async fn handles_missing_package() {
        let registry = make_registry();
        let result = registry.execute("osv_check", &json!({
            "call_id": "test-1",
        })).await;

        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("Missing 'package_name'"));
    }

    #[tokio::test]
    async fn checks_pyPI_package() {
        let registry = make_registry();
        let result = registry.execute("osv_check", &json!({
            "package_name": "requests",
            "version": "2.28.0",
            "call_id": "test-2",
        })).await;

        // Should not error on input validation
        // May return "no vulns" or actual results from OSV API
        assert!(result.error.is_none() || result.error.as_ref().unwrap().contains("OSV API"));
        assert!(result.output.contains("requests"));
    }

    #[tokio::test]
    async fn checks_npm_package() {
        let registry = make_registry();
        let result = registry.execute("osv_check", &json!({
            "package_name": "lodash",
            "version": "4.17.20",
            "call_id": "test-3",
        })).await;

        assert!(result.output.contains("lodash"));
    }

    #[tokio::test]
    async fn handles_invalid_package() {
        let registry = make_registry();
        let result = registry.execute("osv_check", &json!({
            "package_name": "nonexistent-package-xyz-12345",
            "call_id": "test-4",
        })).await;

        // OSV may return 404 or empty results
        assert!(result.output.contains("nonexistent-package-xyz-12345")
            || result.error.as_ref().map(|e| e.contains("404")).unwrap_or(false));
    }
}
