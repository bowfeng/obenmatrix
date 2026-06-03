//! Security advisory checker for supply-chain attacks.
//!
//! Inspired by [`hermes-agent/security_advisories.py`].
//!
//! # Design Goals
//!
//! - **Cheap.** Scans installed packages via a configurable lookup function.
//!   Safe to run on every CLI startup.
//! - **Quiet unless needed.** If no compromised package is detected, the
//!   user sees nothing.
//! - **Extensible.** Adding a new advisory is adding one `Advisory` struct —
//!   no code changes needed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;

// ---- Advisory data model ----

/// Severity level of a security advisory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }
}

/// One security advisory entry describing a known-compromised package or
/// set of package versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub compromised: Vec<CompromisedPackage>,
    pub remediation: Vec<String>,
    pub published: String,
    pub severity: Severity,
}

/// A package and the set of versions known to be compromised.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompromisedPackage {
    pub name: String,
    pub bad_versions: BTreeSet<String>,
}

/// A match between an installed package version and an advisory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisoryHit {
    pub advisory: Advisory,
    pub package: String,
    pub installed_version: String,
}

// ---- Advisory catalog ----

/// Returns all built-in security advisories.
pub fn get_advisories() -> Vec<Advisory> {
    let mut ads: Vec<Advisory> = Vec::new();
    ads.push(advisory_shai_hulud());
    ads
}

fn advisory_shai_hulud() -> Advisory {
    let mut bad = BTreeSet::new();
    bad.insert("2.4.6".to_string());

    Advisory {
        id: "shai-hulud-2026-05".to_string(),
        title: "Mini Shai-Hulud worm — mistralai 2.4.6 compromised on PyPI".to_string(),
        summary: "PyPI quarantined the mistralai package on 2026-05-12 after a malicious \
             2.4.6 release. The worm steals credentials from environment variables \
             and credential files (~/.npmrc, ~/.pypirc, ~/.aws/credentials, GitHub \
             PATs, cloud SDK tokens) and exfils them to a hardcoded webhook."
            .to_string(),
        url: "https://socket.dev/blog/mini-shai-hulud-worm-pypi".to_string(),
        compromised: vec![CompromisedPackage {
            name: "mistralai".to_string(),
            bad_versions: bad,
        }],
        remediation: vec![
            "Run `cargo uninstall mistralai` or `pip uninstall -y mistralai`".to_string(),
            "Rotate all API keys and tokens (OpenRouter, Anthropic, AWS, etc.).".to_string(),
            "Audit ~/.npmrc, ~/.pypirc, ~/.aws/credentials, GitHub PATs for tokens \
             that may have been exposed."
                .to_string(),
            "Check GitHub for unexpected SSH keys, deploy keys, or webhook \
             additions on repos you administer."
                .to_string(),
        ],
        published: "2026-05-12".to_string(),
        severity: Severity::Critical,
    }
}

// ---- Detection ----

/// Installed package version lookup function.
pub type VersionLookup = fn(&str) -> Option<String>;

/// Scan installed packages against the advisory catalog.
///
/// # Arguments
///
/// * `lookup` — A function that returns the installed version of a package
///   name, or `None` if not installed.
/// * `advisories` — Optional override list. If `None`, uses the built-in catalog.
pub fn detect_compromised<F>(lookup: F, advisories: Option<&[Advisory]>) -> Vec<AdvisoryHit>
where
    F: Fn(&str) -> Option<String>,
{
    let ads = advisories.unwrap_or_else(|| {
        let cached = get_advisories();
        let slice: &'static [Advisory] = Box::leak(cached.into_boxed_slice());
        slice
    });

    let mut hits: Vec<AdvisoryHit> = Vec::new();

    for advisory in ads {
        for pkg in &advisory.compromised {
            let installed = match lookup(&pkg.name) {
                Some(v) => v,
                None => continue,
            };

            let matched = pkg.bad_versions.is_empty() || pkg.bad_versions.contains(&installed);

            if matched {
                hits.push(AdvisoryHit {
                    advisory: advisory.clone(),
                    package: pkg.name.clone(),
                    installed_version: installed,
                });
            }
        }
    }

    hits
}

// ---- Rendering helpers ----

/// Format a severity label with optional ANSI coloring.
pub fn severity_label(severity: &Severity, color: bool) -> String {
    let upper = severity.as_str().to_uppercase();
    if color {
        let code = match severity {
            Severity::Low => "\x1b[36m",
            Severity::Medium => "\x1b[33m",
            Severity::High => "\x1b[31m",
            Severity::Critical => "\x1b[1;31m",
        };
        format!("{} {} {}", code, upper, "\x1b[0m")
    } else {
        upper
    }
}

/// Render a short startup banner line for a single hit.
pub fn short_banner(hit: &AdvisoryHit) -> String {
    format!(
        "[{}] {}: {}=={}",
        hit.advisory.id, hit.advisory.title, hit.package, hit.installed_version,
    )
}

/// Render remediation text for a hit.
pub fn remediation_text(hit: &AdvisoryHit) -> String {
    let mut out = String::new();
    writeln!(out, "=== {} ===", hit.advisory.title).ok();
    writeln!(
        out,
        "ID: {}  Severity: {}  Published: {}",
        hit.advisory.id,
        hit.advisory.severity.as_str(),
        hit.advisory.published
    )
    .ok();
    writeln!(out, "Detected: {}=={}", hit.package, hit.installed_version).ok();
    writeln!(out, "Reference: {}", hit.advisory.url).ok();
    writeln!(out).ok();
    writeln!(out, "{}", hit.advisory.summary).ok();
    writeln!(out).ok();
    writeln!(out, "Remediation:").ok();
    for (i, step) in hit.advisory.remediation.iter().enumerate() {
        writeln!(out, "  {}. {}", i + 1, step).ok();
    }
    out
}

/// Render a full doctor-style report for multiple hits.
pub fn render_report(hits: &[AdvisoryHit], color: bool) -> (bool, String) {
    if hits.is_empty() {
        return (
            false,
            "No active security advisories detected.\n".to_string(),
        );
    }

    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let banner = if color {
            let sev_col = match hit.advisory.severity {
                Severity::Low => "\x1b[36m",
                Severity::Medium => "\x1b[33m",
                Severity::High | Severity::Critical => "\x1b[1;31m",
            };
            format!(
                "{}[SECURITY ADVISORY: {}] {}",
                sev_col, hit.advisory.title, "\x1b[0m"
            )
        } else {
            format!("[SECURITY ADVISORY: {}]", hit.advisory.title)
        };
        out.push_str(&banner);
        out.push('\n');
        writeln!(out, "  Found: {}=={}", hit.package, hit.installed_version).ok();
        out.push_str(&remediation_text(hit));
        out.push('\n');
    }

    (true, out)
}

/// Render a concise gateway log message for a single hit.
pub fn gateway_log_message(hit: &AdvisoryHit) -> String {
    format!(
        "Security advisory [{}] active: {}=={} matches {}. See {}",
        hit.advisory.id, hit.package, hit.installed_version, hit.advisory.title, hit.advisory.url,
    )
}

/// Render a gateway log message for multiple hits.
pub fn gateway_log_messages(hits: &[AdvisoryHit]) -> String {
    if hits.len() == 1 {
        return gateway_log_message(&hits[0]);
    }
    if hits.is_empty() {
        return "".to_string();
    }

    let mut ids: Vec<&str> = Vec::new();
    for h in hits {
        ids.push(h.advisory.id.as_str());
    }
    format!(
        "{} security advisories active (IDs: {}). Run hermes doctor for details.",
        hits.len(),
        ids.join(", ")
    )
}

// ---- OSV malware check ----

/// Check if an MCP server package has known malware via the OSV API.
///
/// Queries [Google's OSV API](https://osv.dev) for `MAL-*` (malware) advisory
/// IDs only. Returns `Some(error_msg)` if malware detected, `None` (allow)
/// on clean / network error / unrecognized command.
///
/// Supports `npx`/`npx.cmd` → npm and `uvx`/`uvx.cmd`/`pipx` → PyPI.
/// Fails open on network errors.
pub fn check_osv_malware(command: &str, args: &[String]) -> Option<String> {
    let ecosystem = infer_osv_ecosystem(command);
    if ecosystem.is_none() {
        return None;
    }
    let eco = ecosystem.unwrap();
    let (package, version) = parse_package_from_args(args, eco);

    if package.is_none() {
        return None;
    }

    query_osv(&package.unwrap(), eco, version.as_deref())
}

const OSV_ENDPOINT: &str = "https://api.osv.dev/v1/query";
const OSV_TIMEOUT_SECS: u64 = 10;

fn infer_osv_ecosystem(command: &str) -> Option<&'static str> {
    let base = command
        .trim_end_matches('/')
        .split('/')
        .last()
        .unwrap_or("");
    let base = base.to_lowercase();
    match base.as_str() {
        "npx" | "npx.cmd" => Some("npm"),
        "uvx" | "uvx.cmd" | "pipx" => Some("PyPI"),
        _ => None,
    }
}

fn parse_package_from_args(args: &[String], ecosystem: &str) -> (Option<String>, Option<String>) {
    let package_token = args.iter().find(|a| !a.starts_with('-')).cloned();
    let token = match package_token {
        Some(t) => t,
        None => return (None, None),
    };

    if ecosystem == "npm" {
        return parse_npm_package(&token);
    }
    if ecosystem == "PyPI" {
        return parse_pypi_package(&token);
    }
    (Some(token), None)
}

fn parse_npm_package(token: &str) -> (Option<String>, Option<String>) {
    if token.starts_with('@') {
        let parts: Vec<&str> = token.rsplitn(2, '@').collect();
        if parts.len() == 2 {
            // rsplit gives [version, name]
            let name = parts[1];
            let version = if parts[0] != "latest" {
                Some(parts[0].to_string())
            } else {
                None
            };
            return (Some(name.to_string()), version);
        }
    }
    if let Some(pos) = token.find('@') {
        let name = &token[..pos];
        let rest = &token[pos + 1..];
        if !rest.is_empty() && rest != "latest" {
            return (Some(name.to_string()), Some(rest.to_string()));
        }
    }
    (Some(token.to_string()), None)
}

fn parse_pypi_package(token: &str) -> (Option<String>, Option<String>) {
    let clean: String = token.split('[').next().unwrap_or(token).to_string();
    let parts: Vec<&str> = clean.splitn(2, "==").collect();
    if parts.len() == 2 {
        return (Some(parts[0].to_string()), Some(parts[1].to_string()));
    }
    (Some(clean), None)
}

fn query_osv(package: &str, ecosystem: &str, version: Option<&str>) -> Option<String> {
    let mut payload = serde_json::Map::new();
    let mut pkg = serde_json::Map::new();
    pkg.insert("name".to_string(), serde_json::json!(package));
    pkg.insert("ecosystem".to_string(), serde_json::json!(ecosystem));
    payload.insert("package".to_string(), serde_json::Value::Object(pkg));
    if let Some(v) = version {
        payload.insert("version".to_string(), serde_json::json!(v));
    }

    let resp = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(OSV_TIMEOUT_SECS))
        .build()
        .ok()?
        .post(OSV_ENDPOINT)
        .header("Content-Type", "application/json")
        .header("User-Agent", "oben-agent-osv-check/1.0")
        .json(&serde_json::Value::Object(payload))
        .send()
    {
        Ok(r) => r,
        Err(_) => return None,
    };

    let body: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(_) => return None,
    };

    let vulns: Vec<serde_json::Value> = body
        .get("vulns")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let malware: Vec<_> = vulns
        .iter()
        .filter(|v| {
            let id = v.get("id").and_then(|id| id.as_str()).unwrap_or("");
            id.starts_with("MAL-")
        })
        .collect();

    if malware.is_empty() {
        return None;
    }

    let ids: Vec<String> = malware
        .iter()
        .take(3)
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()).map(String::from))
        .collect();

    let summaries: Vec<String> = malware
        .iter()
        .take(3)
        .filter_map(|v| v.get("summary").and_then(|s| s.as_str()))
        .map(|s| {
            let limit = s.len().min(100);
            s[..limit].to_string()
        })
        .collect();

    Some(format!(
        "BLOCKED: Package '{}' ({}) has known malware advisories: {}. Details: {}",
        package,
        ecosystem,
        ids.join(", "),
        summaries.join("; "),
    ))
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_table(t: &[(&str, Option<String>)]) -> impl Fn(&str) -> Option<String> {
        let map: Vec<(String, Option<String>)> =
            t.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        move |name: &str| {
            map.iter()
                .find(|(k, _)| k.as_str() == name)
                .map(|(_, v)| v.clone())
                .flatten()
        }
    }

    /// Given no matching packages, When detect_compromised runs,
    /// Then returns an empty list.
    #[test]
    fn test_detect_compromised_empty() {
        let hits = detect_compromised(lookup_table(&[("nonexistent", None)]), None);
        assert!(hits.is_empty());
    }

    /// Given a package with a compromised version, When detect_compromised runs,
    /// Then returns a hit with correct details.
    #[test]
    fn test_detect_compromised_match() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].package, "mistralai");
        assert_eq!(hits[0].installed_version, "2.4.6");
        assert_eq!(hits[0].advisory.id, "shai-hulud-2026-05");
    }

    /// Given a package with a safe version, When detect_compromised runs,
    /// Then returns no hit.
    #[test]
    fn test_detect_compromised_safe_version() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.5".to_string()))]),
            None,
        );
        assert!(hits.is_empty());
    }

    /// Given an empty compromised set (wildcard), When detect_compromised runs
    /// with the package installed, Then returns a hit for any version.
    #[test]
    fn test_detect_compromised_wildcard() {
        let wildcard = vec![Advisory {
            id: "wildcard-test".to_string(),
            title: "Compromised namespace".to_string(),
            summary: "x".to_string(),
            url: "https://example.com".to_string(),
            compromised: vec![CompromisedPackage {
                name: "evil-ns".to_string(),
                bad_versions: BTreeSet::new(),
            }],
            remediation: vec!["uninstall it".to_string()],
            published: "2026-01-01".to_string(),
            severity: Severity::High,
        }];
        let hits = detect_compromised(
            lookup_table(&[("evil-ns", Some("0.0.1".to_string()))]),
            Some(&wildcard),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].installed_version, "0.0.1");
    }

    /// Given multiple installed packages, When detect_compromised runs,
    /// Then returns only matching hits.
    #[test]
    fn test_detect_compromised_multiple_hits() {
        let hits = detect_compromised(
            lookup_table(&[
                ("mistralai", Some("2.4.6".to_string())),
                ("safe-pkg", Some("1.0.0".to_string())),
            ]),
            None,
        );
        assert_eq!(hits.len(), 1);
    }

    /// Given no hits, When render_report runs,
    /// Then returns (false, "No active security advisories...").
    #[test]
    fn test_render_report_empty() {
        let (has_problems, text) = render_report(&[], false);
        assert!(!has_problems);
        assert!(text.contains("No active security advisories"));
    }

    /// Given a hit, When render_report runs with color=true,
    /// Then the output contains ANSI color codes.
    #[test]
    fn test_render_report_with_color() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        let (has_problems, text) = render_report(&hits, true);
        assert!(has_problems);
        assert!(text.contains('\x1b'));
        assert!(text.contains("mistralai"));
    }

    /// Given a hit, When render_report runs with color=false,
    /// Then the output is plain text without escape codes.
    #[test]
    fn test_render_report_no_color() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        let (_has_problems, text) = render_report(&hits, false);
        assert!(!text.contains('\x1b'));
    }

    /// Given a hit, When short_banner runs,
    /// Then it includes id, title, package and version.
    #[test]
    fn test_short_banner() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        let banner = short_banner(&hits[0]);
        assert!(banner.contains("shai-hulud-2026-05"));
        assert!(banner.contains("mistralai"));
        assert!(banner.contains("2.4.6"));
    }

    /// Given a hit, When remediation_text runs,
    /// Then it contains all remediation steps.
    #[test]
    fn test_remediation_text_contains_steps() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        let text = remediation_text(&hits[0]);
        for step in &hits[0].advisory.remediation {
            assert!(text.contains(step));
        }
        assert!(text.contains("1."));
        assert!(text.contains(hits[0].advisory.url.as_str()));
    }

    /// Given Severity::Critical, When severity_label runs with color,
    /// Then the output contains ANSI codes and the severity name.
    #[test]
    fn test_severity_label_colored() {
        let label = severity_label(&Severity::Critical, true);
        assert!(label.contains('\x1b'));
        assert!(label.to_uppercase().contains("CRITICAL"));
    }

    /// Given Severity levels, When severity_label runs without color,
    /// Then it returns plain uppercase text.
    #[test]
    fn test_severity_label_plain() {
        assert_eq!(severity_label(&Severity::Low, false), "LOW");
        assert_eq!(severity_label(&Severity::Medium, false), "MEDIUM");
        assert_eq!(severity_label(&Severity::High, false), "HIGH");
        assert_eq!(severity_label(&Severity::Critical, false), "CRITICAL");
    }

    /// When get_advisories runs, it returns at least one advisory
    /// with all required fields non-empty.
    #[test]
    fn test_get_advisories() {
        let ads = get_advisories();
        assert!(!ads.is_empty());
        for a in &ads {
            assert!(!a.id.is_empty());
            assert!(!a.title.is_empty());
            assert!(!a.summary.is_empty());
            assert!(a.url.starts_with("http"));
            assert!(!a.compromised.is_empty());
            assert!(!a.remediation.is_empty());
        }
    }

    /// Given a custom Advisory, When detect_compromised runs with custom advisories,
    /// Then the custom logic works correctly.
    #[test]
    fn test_custom_advisory_rendering() {
        let custom = vec![Advisory {
            id: "custom-1".to_string(),
            title: "Custom advisory".to_string(),
            summary: "Test summary text".to_string(),
            url: "https://custom.example.com/advisory".to_string(),
            compromised: vec![CompromisedPackage {
                name: "evil-pkg".to_string(),
                bad_versions: {
                    let mut s = BTreeSet::new();
                    s.insert("1.0.0".to_string());
                    s
                },
            }],
            remediation: vec!["uninstall evil-pkg".to_string(), "rotate keys".to_string()],
            published: "2026-03-01".to_string(),
            severity: Severity::High,
        }];
        let hits = detect_compromised(
            lookup_table(&[("evil-pkg", Some("1.0.0".to_string()))]),
            Some(&custom),
        );
        assert_eq!(hits.len(), 1);
        let text = remediation_text(&hits[0]);
        assert!(text.contains("Custom advisory"));
        assert!(text.contains("evil-pkg"));
        assert!(text.contains("2026-03-01"));
        assert!(text.contains("1. uninstall evil-pkg"));
    }

    /// Given multiple hits, When gateway_log_messages runs,
    /// Then it returns a summary with all IDs.
    #[test]
    fn test_gateway_log_messages_multi() {
        let ads = vec![
            Advisory {
                id: "adv-1".to_string(),
                title: "A1".to_string(),
                summary: "x".to_string(),
                url: "https://a.com".to_string(),
                compromised: vec![CompromisedPackage {
                    name: "p1".to_string(),
                    bad_versions: {
                        let mut s = BTreeSet::new();
                        s.insert("1.0".to_string());
                        s
                    },
                }],
                remediation: vec!["uninstall p1".to_string()],
                published: "2026-01-01".to_string(),
                severity: Severity::High,
            },
            Advisory {
                id: "adv-2".to_string(),
                title: "A2".to_string(),
                summary: "y".to_string(),
                url: "https://b.com".to_string(),
                compromised: vec![CompromisedPackage {
                    name: "p2".to_string(),
                    bad_versions: {
                        let mut s = BTreeSet::new();
                        s.insert("2.0".to_string());
                        s
                    },
                }],
                remediation: vec!["uninstall p2".to_string()],
                published: "2026-02-01".to_string(),
                severity: Severity::Medium,
            },
        ];
        let hits = detect_compromised(
            lookup_table(&[
                ("p1", Some("1.0".to_string())),
                ("p2", Some("2.0".to_string())),
            ]),
            Some(&ads),
        );
        assert_eq!(hits.len(), 2);
        let msg = gateway_log_messages(&hits);
        assert!(msg.contains("2 security advisories"));
        assert!(msg.contains("adv-1"));
        assert!(msg.contains("adv-2"));
    }

    /// Given a single hit, When gateway_log_messages runs,
    /// Then it returns the single-hit format with package name.
    #[test]
    fn test_gateway_log_message_single() {
        let hits = detect_compromised(
            lookup_table(&[("mistralai", Some("2.4.6".to_string()))]),
            None,
        );
        let msg = gateway_log_messages(&hits);
        assert!(msg.contains("mistralai"));
        assert!(msg.contains("shai-hulud-2026-05"));
    }

    /// Given an empty hit list, When gateway_log_messages runs,
    /// Then it returns an empty string.
    #[test]
    fn test_gateway_log_messages_empty() {
        let msg = gateway_log_messages(&[]);
        assert_eq!(msg, "");
    }
}
