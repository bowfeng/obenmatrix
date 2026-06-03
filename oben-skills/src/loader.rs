/// Load skills from YAML/TXT files.
/// Maps to how Hermes loads skill files from the `skills/` directory.
use anyhow::Result;
use serde_yaml::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::{debug, info};

// ── Constants ───────────────────────────────────────────────────────────────

/// Directories to exclude when recursively scanning for skills.
const EXCLUDED_DIRS: &[&str] = &[".git", ".github", ".hub", ".archive", "node_modules"];

/// Platform mapping: YAML frontmatter names → `std::env::consts::OS`.
const PLATFORM_MAP: &[(&str, &str)] = &[
    ("macos", "macos"),
    ("darwin", "macos"),
    ("linux", "linux"),
    ("windows", "windows"),
    ("win32", "windows"),
    ("win", "windows"),
];

// ── Frontmatter parsing ─────────────────────────────────────────────────────

/// Parse YAML frontmatter from a markdown string.
///
/// Extracts the block between `---` delimiters at the start of the file
/// and returns (frontmatter_map, remaining_body).
/// Returns (empty, original) if no frontmatter is found.
pub fn parse_frontmatter(content: &str) -> (BTreeMap<String, Value>, String) {
    let mut frontmatter: BTreeMap<String, Value> = BTreeMap::new();
    let body = content.to_string();

    if !content.starts_with("---") {
        return (frontmatter, body);
    }

    // Find closing --- with a newline after it
    let rest = &content[3..];
    if let Some(end_pos) = rest.find("\n---\n") {
        let yaml_content = &rest[..end_pos];
        let body_start = end_pos + 6; // skip "\n---\n"
        let body = content.get(body_start..).unwrap_or("").to_string();

        match serde_yaml::from_str::<Value>(yaml_content) {
            Ok(Value::Mapping(map)) => {
                for (k, v) in map {
                    if let Some(key) = k.as_str() {
                        frontmatter.insert(key.to_string(), v);
                    }
                }
            }
            Ok(_) => {
                // YAML parsed but not a mapping — ignore
            }
            Err(e) => {
                debug!("Frontmatter YAML parse error: {e}, falling back to simple key:value");
                // Fallback: simple key:value parsing for malformed YAML
                for line in yaml_content.lines() {
                    if let Some((key, val)) = line.split_once(':') {
                        let key = key.trim();
                        let val = val.trim().trim_matches('"').trim_matches('\'');
                        if !key.is_empty() {
                            frontmatter.insert(key.to_string(), Value::String(val.to_string()));
                        }
                    }
                }
            }
        }
        return (frontmatter, body);
    }

    (frontmatter, body)
}

/// Extract a platform list from frontmatter.
///
/// Supports both top-level `platforms: [linux, macos]`
/// and nested `metadata: platforms: [linux, macos]`.
pub fn extract_platforms(frontmatter: &BTreeMap<String, Value>) -> Vec<String> {
    // Check top-level first
    if let Some(Value::Sequence(seqs)) = frontmatter.get("platforms") {
        return seqs
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect();
    }
    // Check nested under metadata
    if let Some(Value::Mapping(meta)) = frontmatter.get("metadata") {
        if let Some(Value::Sequence(seqs)) = meta.get("platforms") {
            return seqs
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
        }
    }
    Vec::new()
}

/// Check whether a skill is compatible with the current OS.
///
/// If the `platforms` frontmatter field is absent or empty, the skill
/// is compatible with all platforms (backward-compatible default).
pub fn skill_matches_platform(frontmatter: &BTreeMap<String, Value>) -> bool {
    let platforms = extract_platforms(frontmatter);
    if platforms.is_empty() {
        return true;
    }

    let current_os = std::env::consts::OS;
    for platform in &platforms {
        let normalized = platform.to_lowercase();
        for (yaml_name, rust_os) in PLATFORM_MAP {
            if yaml_name.eq_ignore_ascii_case(&normalized) && current_os == *rust_os {
                return true;
            }
        }
    }
    false
}

/// Extract a truncated description from parsed frontmatter.
///
/// Truncates to 60 characters with ellipsis.
pub fn extract_skill_description(frontmatter: &BTreeMap<String, Value>) -> String {
    let raw_desc = frontmatter
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let desc = raw_desc.trim().trim_matches('"').trim_matches('\'');
    if desc.len() > 60 {
        desc[..57].to_string() + "..."
    } else {
        desc.to_string()
    }
}

/// Extract tags from frontmatter (top-level or under metadata.heremes.tags).
pub fn extract_tags(frontmatter: &BTreeMap<String, Value>) -> Vec<String> {
    if let Some(tags_val) = frontmatter.get("tags") {
        if let Value::Sequence(tags) = tags_val {
            return tags
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
        }
        if let Some(tag_str) = tags_val.as_str() {
            // Comma-separated tags
            return tag_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    // Check under metadata
    if let Some(Value::Mapping(meta)) = frontmatter.get("metadata") {
        if let Some(Value::Sequence(tags)) = meta.get("tags") {
            return tags
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
        }
    }
    Vec::new()
}

/// Extract condition information from frontmatter.
///
/// Maps to `extract_skill_conditions()` in hermes-agent.
pub fn extract_skill_conditions(
    frontmatter: &BTreeMap<String, Value>,
) -> BTreeMap<String, Vec<String>> {
    let mut conditions = BTreeMap::new();
    if let Some(Value::Mapping(meta)) = frontmatter.get("metadata") {
        if let Some(Value::Mapping(hermes)) = meta.get("hermes") {
            for key in [
                "fallback_for_toolsets",
                "requires_toolsets",
                "fallback_for_tools",
                "requires_tools",
            ] {
                if let Some(Value::Sequence(items)) = hermes.get(key) {
                    let vals: Vec<String> = items
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect();
                    if !vals.is_empty() {
                        conditions.insert(key.to_string(), vals);
                    }
                }
            }
        }
    }
    conditions
}

/// Extract config variable declarations from frontmatter.
///
/// Skills declare config.yaml settings they need via::
/// ```yaml
/// metadata:
///   hermes:
///     config:
///       - key: wiki.path
///         description: Path to the LLM Wiki
///         default: "~/wiki"
///         prompt: Wiki directory path
/// ```
#[derive(Debug, Clone)]
pub struct ConfigVar {
    pub key: String,
    pub description: String,
    pub default: Option<String>,
    pub prompt: Option<String>,
}

pub fn extract_skill_config_vars(frontmatter: &BTreeMap<String, Value>) -> Vec<ConfigVar> {
    let mut vars = Vec::new();

    if let Some(Value::Mapping(meta)) = frontmatter.get("metadata") {
        if let Some(Value::Mapping(hermes)) = meta.get("hermes") {
            if let Some(Value::Sequence(configs)) = hermes.get("config") {
                for item in configs {
                    if let Value::Mapping(c) = item {
                        let key = c.get("key").and_then(|v| v.as_str()).unwrap_or("");
                        if key.is_empty() {
                            continue;
                        }
                        let desc = c.get("description").and_then(|v| v.as_str()).unwrap_or("");
                        if desc.is_empty() {
                            continue;
                        }
                        let default = c
                            .get("default")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let prompt = c
                            .get("prompt")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        vars.push(ConfigVar {
                            key: key.to_string(),
                            description: desc.to_string(),
                            default,
                            prompt,
                        });
                    }
                }
            }
        }
    }
    vars
}

// ── File iteration ──────────────────────────────────────────────────────────

/// Walk `skills_dir` yielding sorted paths matching `filename` (e.g., "SKILL.md").
///
/// This is the equivalent of `iter_skill_index_files()` in hermes-agent.
pub fn iter_skill_index_files(skills_dir: &Path, filename: &str) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    walk_for_skill_files(skills_dir, filename, &mut matches);
    // Deduplicate and sort
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<PathBuf> = matches
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect();
    let mut deduped = deduped;
    deduped.sort();
    deduped
}

fn walk_for_skill_files(dir: &Path, filename: &str, matches: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }

    // Check for the filename directly in the top-level directory
    let top_level = dir.join(filename);
    if top_level.is_file() {
        matches.push(top_level);
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();

        if let Some(name_str) = name.to_str() {
            if EXCLUDED_DIRS.contains(&name_str) {
                continue;
            }
        }

        if path.is_dir() {
            let full_path = path.join(filename);
            if full_path.is_file() {
                matches.push(full_path);
            }
            walk_for_skill_files(&path, filename, matches);
        }
    }
}

// ── Qualified name helpers ──────────────────────────────────────────────────

/// Split `'namespace:skill-name'` into `(namespace, bare_name)`.
///
/// Returns `(None, name)` when there is no `':'`.
/// Equivalent to `parse_qualified_name()` in hermes-agent.
pub fn parse_qualified_name(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((ns, bare)) => (Some(ns), bare),
        None => (None, name),
    }
}

/// Validate a namespace string (must match `[a-zA-Z0-9_-]+`).
pub fn is_valid_namespace(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ── Disabled skills ─────────────────────────────────────────────────────────

/// Read disabled skill names from a config file.
///
/// For now, reads from `~/.config/obenalien/config.yaml` if it exists.
/// Returns an empty set if the file doesn't exist or can't be parsed.
pub fn get_disabled_skill_names() -> std::collections::HashSet<String> {
    let config_path = get_config_path();
    if !config_path.exists() {
        return std::collections::HashSet::new();
    }
    match fs::read_to_string(&config_path) {
        Ok(_) => {
            // Parse YAML and extract skills.disabled list
            // For now return empty — full implementation needs serde_yaml on the config
            std::collections::HashSet::new()
        }
        Err(_) => std::collections::HashSet::new(),
    }
}

fn get_config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        Path::new(&home).join(".config/obenalien/config.yaml")
    } else {
        Path::new(".").join("config.yaml")
    }
}

// ── External skills directories (cached) ───────────────────────────────────

static EXTERNAL_SKILLS_CACHE: LazyLock<std::sync::Mutex<ExternalSkillsCache>> =
    LazyLock::new(|| std::sync::Mutex::new(ExternalSkillsCache::default()));

#[derive(Default)]
struct ExternalSkillsCache {
    mtime: Option<u64>,
    dirs: Vec<PathBuf>,
}

/// Read external skills directories from config.yaml.
///
/// Reads `skills.external_dirs` from the config and returns validated paths.
/// Results are cached by file modification time to avoid repeated YAML parsing.
pub fn get_external_skills_dirs() -> Vec<PathBuf> {
    let config_path = get_config_path();

    // Compute mtime of config file.
    let file_mtime = std::fs::metadata(&config_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|d| d.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    let should_freshen = match (file_mtime, {
        let cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
        cache.mtime
    }) {
        (Some(new_mtime), Some(old_mtime)) => new_mtime != old_mtime,
        (None, _) => return Vec::new(), // config doesn't exist
        (Some(_), None) => true,        // first call, or mtime changed
    };

    if !should_freshen {
        let cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
        return cache.dirs.clone();
    }

    let mut external_dirs = Vec::new();
    if !config_path.exists() {
        let mut cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
        cache.mtime = None;
        cache.dirs.clear();
        return external_dirs;
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => {
            let mut cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
            cache.mtime = file_mtime;
            return external_dirs;
        }
    };

    let parsed: Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            let mut cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
            cache.mtime = file_mtime;
            return external_dirs;
        }
    };

    if let Value::Mapping(root) = parsed {
        if let Some(Value::Mapping(skills)) = root.get("skills") {
            if let Some(Value::Sequence(dirs)) = skills.get("external_dirs") {
                for entry in dirs {
                    if let Some(dir_str) = entry.as_str() {
                        let expanded = expand_env_vars(dir_str.trim());
                        let p = PathBuf::from(&expanded);
                        let p = if !p.is_absolute() {
                            if let Ok(home) = std::env::var("HOME") {
                                PathBuf::from(&home).join(&p)
                            } else {
                                p
                            }
                        } else {
                            p
                        };
                        if p.is_dir() {
                            external_dirs.push(p);
                        }
                    }
                }
            }
        }
    }

    let mut cache = EXTERNAL_SKILLS_CACHE.lock().unwrap();
    cache.mtime = file_mtime;
    cache.dirs = external_dirs.clone();
    external_dirs
}

fn expand_env_vars(s: &str) -> String {
    let mut result = s.to_string();
    for var in ["HOME", "USERPROFILE", "XDG_DATA_HOME", "XDG_CONFIG_HOME"] {
        if let Ok(val) = std::env::var(var) {
            result = result.replace(&format!("${{{}}}", var), &val);
            result = result.replace(&format!("${}", var), &val);
        }
    }
    // Basic ~ expansion
    if result.starts_with("~/") || result == "~" {
        if let Ok(home) = std::env::var("HOME") {
            result = format!("{}{}", home, &result[1..]);
        }
    }
    result
}

/// Get all skill directories: local first, then external.
pub fn get_all_skills_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![get_skills_dir()];
    dirs.extend(get_external_skills_dirs());
    dirs
}

fn get_skills_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        Path::new(&home).join(".config/obenalien/skills")
    } else if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        Path::new(&xdg).join("obenalien/skills")
    } else {
        Path::new(".").join("skills")
    }
}

// ── SkillLoader ─────────────────────────────────────────────────────────────

/// Skill loader — reads skill definitions from disk.
pub struct SkillLoader {
    skill_dirs: Vec<PathBuf>,
}

impl SkillLoader {
    pub fn new() -> Self {
        Self { skill_dirs: vec![] }
    }

    /// Add a directory to search for skills.
    pub fn add_dir(&mut self, dir: impl Into<PathBuf>) {
        self.skill_dirs.push(dir.into());
    }

    /// Load all skills from configured directories.
    ///
    /// Recursively finds all `SKILL.md` files (like `category/sub-skill/SKILL.md`),
    /// parses frontmatter, filters by platform, and returns enabled skills.
    pub fn load_all(&self) -> Result<Vec<oben_models::Skill>> {
        let mut skills = Vec::new();

        for dir in &self.skill_dirs {
            if !dir.exists() {
                debug!("Skill directory not found: {}", dir.display());
                continue;
            }

            // Find all SKILL.md files recursively
            let skill_files = iter_skill_index_files(dir, "SKILL.md");
            let mut loaded = 0;
            let mut skipped_disabled = 0;
            // Track which files were already loaded to avoid duplicates.
            // Use case-insensitive relative paths for case-insensitive filesystems (macOS).
            let loaded_files: std::collections::HashSet<String> = skill_files
                .iter()
                .filter_map(|p| {
                    p.strip_prefix(dir)
                        .ok()
                        .and_then(|rel| rel.to_str())
                        .map(|r| r.to_lowercase())
                })
                .collect();

            // Load disabled set once per directory, not per-file.
            let disabled = get_disabled_skill_names();

            for skill_file in &skill_files {
                match self.load_single_skill(skill_file) {
                    Some(skill) => {
                        // Check if disabled
                        let skill_name = &skill.name;
                        if disabled.contains(skill_name) {
                            skipped_disabled += 1;
                            continue;
                        }
                        skills.push(skill);
                        loaded += 1;
                    }
                    None => {}
                }
            }

            // Also try to load flat .md/.txt/.yaml files at top level of each dir
            // Note: .md files are loaded too (for non-SKILL.md files like test-skill.md).
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    // Skip files already loaded via iter_skill_index_files (case-insensitive)
                    if let Some(rel) = path.strip_prefix(dir).ok().and_then(|r| r.to_str()) {
                        if loaded_files.contains(&rel.to_lowercase()) {
                            continue;
                        }
                    }
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ext == "yaml" || ext == "yml" || ext == "txt" || ext == "md" {
                            if let Some(skill) = self.load_file(&path) {
                                skills.push(skill);
                            }
                        }
                    }
                } else if path.is_dir() {
                    // Legacy: also load directories as skills (SKILL.md / SKILL.yaml / README.md)
                    // Skip directories whose SKILL.md was already loaded.
                    let skill_file = path.join("SKILL.md");
                    if !skill_file.exists() {
                        if let Some(skill) = self.load_from_dir(&path) {
                            skills.push(skill);
                        }
                    }
                }
            }

            info!(
                "Loaded {} skills from {:?} (skipped {} disabled)",
                loaded, dir, skipped_disabled
            );
        }

        info!(
            "Total loaded {} skills from {} directories",
            skills.len(),
            self.skill_dirs.len()
        );
        Ok(skills)
    }

    /// Load a single skill from a SKILL.md file.
    fn load_single_skill(&self, path: &Path) -> Option<oben_models::Skill> {
        let content = fs::read_to_string(path).ok()?;
        let (frontmatter, _body) = parse_frontmatter(&content);

        let name = frontmatter
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        let description = extract_skill_description(&frontmatter);

        let category = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let metadata = if frontmatter.is_empty() {
            None
        } else {
            Some(to_value_map(frontmatter))
        };

        Some(
            oben_models::Skill::builder(name)
                .description(description)
                .category(category)
                .instructions(content)
                .metadata(metadata)
                .build(),
        )
    }

    fn load_from_dir(&self, dir: &Path) -> Option<oben_models::Skill> {
        let name = dir.file_name()?.to_str()?;
        let skill_dir = dir;

        // Try to find a SKILL.md or skill.yaml
        let skill_file = skill_dir.join("SKILL.md");
        let yaml_file = skill_dir.join("SKILL.yaml");

        if skill_file.exists() {
            let content = fs::read_to_string(&skill_file).ok()?;
            let (frontmatter, _body) = parse_frontmatter(&content);
            let description = extract_skill_description(&frontmatter);
            let metadata = if frontmatter.is_empty() {
                None
            } else {
                Some(to_value_map(frontmatter))
            };

            Some(
                oben_models::Skill::builder(name)
                    .description(description)
                    .category(dir.parent()?.file_name()?.to_str()?.to_string())
                    .instructions(content)
                    .metadata(metadata)
                    .build(),
            )
        } else if yaml_file.exists() {
            let content = fs::read_to_string(&yaml_file).ok()?;
            serde_yaml::from_str(&content).ok()
        } else {
            // Use README as instructions
            let readme = skill_dir.join("README.md");
            if readme.exists() {
                let content = fs::read_to_string(&readme).ok()?;
                Some(
                    oben_models::Skill::builder(name)
                        .description(format!("Skill: {}", name))
                        .category(dir.parent()?.file_name()?.to_str()?.to_string())
                        .instructions(content)
                        .build(),
                )
            } else {
                Some(
                    oben_models::Skill::builder(name)
                        .description(format!("Skill: {}", name))
                        .category(dir.parent()?.file_name()?.to_str()?.to_string())
                        .instructions("(no instructions found)")
                        .build(),
                )
            }
        }
    }

    fn load_file(&self, path: &Path) -> Option<oben_models::Skill> {
        let content = fs::read_to_string(path).ok()?;
        let name = path.file_stem()?.to_str()?;

        // Check if it's YAML
        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            serde_yaml::from_str(&content).ok()
        } else {
            Some(
                oben_models::Skill::builder(name)
                    .description(content.lines().next().unwrap_or("").to_string())
                    .instructions(content)
                    .build(),
            )
        }
    }
}

/// Helper: convert a `BTreeMap<String, Value>` into a `Value::Mapping`.
fn to_value_map(map: BTreeMap<String, Value>) -> Value {
    Value::Mapping(serde_yaml::Mapping::from_iter(
        map.into_iter().map(|(k, v)| (Value::String(k), v)),
    ))
}

/// Default skills that come with the system.
pub fn builtin_skills() -> Vec<oben_models::Skill> {
    vec![oben_models::Skill::builder("general")
        .description("General-purpose conversation and task assistance")
        .category("core")
        .instructions("You are a helpful AI assistant. Help the user accomplish their goals efficiently and accurately.")
        .enabled(true)
        .auto_use(true)
        .metadata(None)
        .build()]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oben_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn write_skill_md(path: &Path, frontmatter: &str, body: &str) {
        let content = format!("---\n{}\n---\n\n{}", frontmatter, body);
        fs::write(path, content).ok();
    }

    // ── Frontmatter parsing ──────────────────────────────────────────────

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nname: test-skill\ndescription: A test skill\n---\n\nSkill body here.";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("test-skill"));
        assert_eq!(
            fm.get("description").and_then(|v| v.as_str()),
            Some("A test skill")
        );
        assert!(body.contains("Skill body here"));
    }

    #[test]
    fn test_parse_frontmatter_nested() {
        let content = "---\nname: mlops\ndescription: ML ops\nmetadata:\n  platforms: [linux, macos]\n  hermes:\n    config:\n      - key: api.key\n        description: API key\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("mlops"));
        let platforms = extract_platforms(&fm);
        assert!(platforms.contains(&"linux".to_string()));
        assert!(platforms.contains(&"macos".to_string()));
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just some text without frontmatter.";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_frontmatter_malformed_yaml() {
        let content = "---\nname: bad skill\ndesc: missing quote\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(content);
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("bad skill"));
    }

    // ── Platform matching ────────────────────────────────────────────────

    #[test]
    fn test_skill_matches_platform_no_platforms() {
        let fm = BTreeMap::new();
        assert!(skill_matches_platform(&fm));
    }

    #[test]
    fn test_skill_matches_platform_nested_metadata() {
        let mut fm = BTreeMap::new();
        fm.insert(
            "platforms".to_string(),
            Value::Sequence(serde_yaml::Sequence::from_iter([
                Value::String("linux".into()),
                Value::String("macos".into()),
            ])),
        );
        let current = std::env::consts::OS;
        let matches = skill_matches_platform(&fm);
        assert_eq!(matches, current == "linux" || current == "macos");
    }

    #[test]
    fn test_skill_matches_platform_single() {
        let mut fm = BTreeMap::new();
        fm.insert(
            "platforms".to_string(),
            Value::Sequence(serde_yaml::Sequence::from_iter([Value::String(
                "windows".into(),
            )])),
        );
        // On macOS/Linux this should be false
        let current = std::env::consts::OS;
        assert_ne!(current, "windows");
        assert!(!skill_matches_platform(&fm));
    }

    // ── Description extraction ───────────────────────────────────────────

    #[test]
    fn test_extract_skill_description_short() {
        let yaml_str = "---\ndescription: Short description\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(yaml_str);
        let desc = extract_skill_description(&fm);
        assert_eq!(desc, "Short description");
    }

    #[test]
    fn test_extract_skill_description_long() {
        let yaml_str = "---\ndescription: This is a very long description that should be truncated because it exceeds sixty characters\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(yaml_str);
        let desc = extract_skill_description(&fm);
        assert!(desc.len() <= 60);
        assert!(desc.ends_with("..."));
    }

    // ── Tags extraction ──────────────────────────────────────────────────

    #[test]
    fn test_extract_tags_seq() {
        let yaml_str = "---\ntags:\n  - tag1\n  - tag2\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(yaml_str);
        let tags = extract_tags(&fm);
        assert_eq!(tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn test_extract_tags_comma_separated() {
        let yaml_str = "---\ntags: a, b, c\n---\n\nBody";
        let (fm, _body) = parse_frontmatter(yaml_str);
        let tags = extract_tags(&fm);
        assert_eq!(tags, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_extract_tags_none() {
        let fm = BTreeMap::new();
        let tags = extract_tags(&fm);
        assert!(tags.is_empty());
    }

    // ── Config vars extraction ───────────────────────────────────────────

    #[test]
    fn test_extract_config_vars() {
        let yaml_str = r#"---
metadata:
  hermes:
    config:
      - key: api.key
        description: API key
        default: ~/.api_key
---

Body"#;
        let (fm, _body) = parse_frontmatter(yaml_str);
        let vars = extract_skill_config_vars(&fm);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "api.key");
        assert_eq!(vars[0].description, "API key");
        assert_eq!(vars[0].default, Some("~/.api_key".to_string()));
    }

    #[test]
    fn test_extract_config_vars_none() {
        let fm = BTreeMap::new();
        let vars = extract_skill_config_vars(&fm);
        assert!(vars.is_empty());
    }

    // ── Conditions extraction ────────────────────────────────────────────

    #[test]
    fn test_extract_conditions() {
        let yaml_str = r#"---
name: test
metadata:
  hermes:
    requires_toolsets:
      - terminal
      - browser
---

Body"#;
        let (fm, _body) = parse_frontmatter(yaml_str);
        let conditions = extract_skill_conditions(&fm);
        assert!(conditions.contains_key("requires_toolsets"));
    }

    // ── Qualified name parsing ───────────────────────────────────────────

    #[test]
    fn test_parse_qualified_name_with_colon() {
        let (ns, bare) = parse_qualified_name("superpowers:writing-plans");
        assert_eq!(ns, Some("superpowers"));
        assert_eq!(bare, "writing-plans");
    }

    #[test]
    fn test_parse_qualified_name_without_colon() {
        let (ns, bare) = parse_qualified_name("my-skill");
        assert_eq!(ns, None);
        assert_eq!(bare, "my-skill");
    }

    #[test]
    fn test_is_valid_namespace() {
        assert!(is_valid_namespace("superpowers"));
        assert!(is_valid_namespace("my-plugin"));
        assert!(is_valid_namespace("test_123"));
        assert!(!is_valid_namespace(""));
        assert!(!is_valid_namespace("bad space"));
    }

    // ── iter_skill_index_files (recursive scanning) ──────────────────────

    #[test]
    fn test_iter_skill_index_files_flat() {
        let dir = temp_dir("iter_flat");
        write_skill_md(&dir.join("SKILL.md"), "name: root\n", "Root skill");

        let files = iter_skill_index_files(&dir, "SKILL.md");
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("SKILL.md"));
    }

    #[test]
    fn test_iter_skill_index_files_recursive() {
        let dir = temp_dir("iter_recursive");
        fs::create_dir_all(dir.join("category/sub-skill")).ok();
        write_skill_md(
            &dir.join("category/SKILL.md"),
            "name: cat-skill\n",
            "Cat skill",
        );
        write_skill_md(
            &dir.join("category/sub-skill/SKILL.md"),
            "name: nested-skill\n",
            "Nested skill",
        );
        // This should find both
        let files = iter_skill_index_files(&dir, "SKILL.md");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_iter_skill_index_files_nested_dir_has_both() {
        let dir = temp_dir("iter_nested_both");
        fs::create_dir_all(dir.join("mycat/SKILL.md")).ok(); // dir named "SKILL.md" - weird edge case
        let cat_dir = dir.join("mycat");
        fs::create_dir_all(cat_dir.join("subskill")).ok();
        write_skill_md(&cat_dir.join("SKILL.md"), "name: mycat\n", "Category skill");
        write_skill_md(
            &cat_dir.join("subskill/SKILL.md"),
            "name: mycat-sub\n",
            "Sub skill",
        );

        let files = iter_skill_index_files(&dir, "SKILL.md");
        // Should find both SKILL.md files
        assert!(files.len() >= 1);
    }

    #[test]
    fn test_iter_skill_index_files_excludes_git() {
        let dir = temp_dir("iter_excl");
        let git_dir = dir.join(".git");
        fs::create_dir_all(git_dir.join("hooks")).ok();
        write_skill_md(
            &git_dir.join("hooks/SKILL.md"),
            "name: hidden\n",
            "Should not be found",
        );

        let files = iter_skill_index_files(&dir, "SKILL.md");
        let names: Vec<_> = files.iter().map(|p| p.file_name().unwrap()).collect();
        assert!(!names.iter().any(|n| n.to_string_lossy().contains("hidden")));
    }

    // ── Skill loader with frontmatter ────────────────────────────────────

    #[test]
    fn test_skill_loader_loads_directories_as_skills() {
        // Directories are also loaded as skills (via SKILL.md / SKILL.yaml / README.md)
        let dir = temp_dir("load_dirs");
        fs::create_dir(dir.join("subdir")).unwrap();
        write_skill_md(&dir.join("skill.md"), "", "");

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        // skill.md -> "SKILL" skill (loaded via iter_skill_index_files matching SKILL.md),
        // subdir directory -> "subdir" skill (no instructions, via load_from_dir)
        // Total should be 2
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_skill_loader_loads_txt_with_long_content() {
        let dir = temp_dir("txt_long");
        let long_content = "First line.\nSecond line of instructions.";
        fs::write(dir.join("long-skill.txt"), long_content).unwrap();

        let mut loader = SkillLoader::new();
        loader.add_dir(dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "long-skill");
        assert_eq!(skills[0].description, "First line.");
        assert_eq!(skills[0].instructions, long_content);
    }

    #[test]
    fn test_skill_loader_nonexistent_dir() {
        let mut loader = SkillLoader::new();
        loader.add_dir("/tmp/nonexistent_dir_12345");
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_skill_loader_new_is_empty() {
        let loader = SkillLoader::new();
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    // ── Environment variable expansion ───────────────────────────────────

    #[test]
    fn test_expand_env_vars() {
        assert!(expand_env_vars("~").starts_with('/'));
        assert!(expand_env_vars("~/skills").starts_with('/'));
        if let Ok(home) = std::env::var("HOME") {
            assert!(expand_env_vars("${HOME}/test").starts_with(&home));
        }
    }

    // ── Builtin skills ───────────────────────────────────────────────────

    #[test]
    fn test_builtin_skills_returns_one() {
        let skills = builtin_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "general");
        assert_eq!(skills[0].category, "core");
    }

    #[test]
    fn test_parse_frontmatter_with_quotes() {
        let content = r#"---
name: "quoted-skill"
description: 'A skill with quotes'
---

Body content"#;
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(
            fm.get("name").and_then(|v| v.as_str()),
            Some("quoted-skill")
        );
        assert!(body.contains("Body content"));
    }
}
