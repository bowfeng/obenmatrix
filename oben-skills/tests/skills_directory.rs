/// Integration test to verify the skills directory structure.
use std::path::Path;

#[test]
fn test_skills_directory_exists() {
    // The skills directory should exist relative to the workspace root
    let skills_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../skills"));
    assert!(skills_dir.exists(), "Skills directory should exist");
}

#[test]
fn test_skill_categories_exist() {
    // Verify we have at least 20 skill categories
    let skills_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../skills"));

    let mut categories = Vec::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with(".git") {
                categories.push(entry.file_name());
            }
        }
    }

    assert!(
        categories.len() >= 20,
        "Should have at least 20 skill categories, found {}",
        categories.len()
    );
}

#[test]
fn test_skill_files_exist() {
    // Verify each skill category has a SKILL.md file
    let skills_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../skills"));

    let mut skill_count = 0;
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with(".git") {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    skill_count += 1;
                }
            }
        }
    }

    assert!(
        skill_count >= 20,
        "Should have at least 20 SKILL.md files, found {}",
        skill_count
    );
}

#[test]
fn test_general_skill_exists() {
    let skill_md = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../skills/general/SKILL.md"
    ));
    assert!(skill_md.exists(), "general/SKILL.md should exist");

    let content = std::fs::read_to_string(skill_md).unwrap();
    assert!(
        content.contains("# General"),
        "Should contain # General header"
    );
    assert!(
        content.contains("## Capabilities"),
        "Should contain Capabilities section"
    );
}

#[test]
fn test_skill_categories_loaded() {
    use oben_skills::SkillLoader;
    use std::path::PathBuf;

    let skills_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../skills"));
    let mut loader = SkillLoader::new();
    loader.add_dir(skills_dir);

    let skills = loader.load_all().unwrap();
    assert!(!skills.is_empty(), "Should load at least some skills");
}
