use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use regex::Regex;
use serde::Deserialize;

use crate::app::AppResult;

#[derive(Debug, Clone)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: HashMap<String, String>,
    pub content: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SkillStore {
    skills: HashMap<String, SkillDefinition>,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl SkillStore {
    pub fn discover(workspace_root: &Path, runtime_root: &Path) -> AppResult<Self> {
        let mut skills = HashMap::new();
        let mut seen = HashSet::new();

        load_from_skill_root(&runtime_root.join("skills"), &mut skills, &mut seen)?;

        for root in discovery_roots(workspace_root) {
            for base in [root.join(".opencode/skills"), root.join(".claude/skills"), root.join(".agents/skills")] {
                load_from_skill_root(&base, &mut skills, &mut seen)?;
            }
        }

        if let Some(home) = env::var_os("HOME") {
            let home = PathBuf::from(home);
            for base in [
                home.join(".config/opencode/skills"),
                home.join(".claude/skills"),
                home.join(".agents/skills"),
            ] {
                load_from_skill_root(&base, &mut skills, &mut seen)?;
            }
        }

        Ok(Self { skills })
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&SkillDefinition> {
        self.skills.get(name)
    }

    pub fn list(&self) -> Vec<&SkillDefinition> {
        let mut skills = self.skills.values().collect::<Vec<_>>();
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        skills
    }

    pub fn summary_block(&self) -> String {
        if self.skills.is_empty() {
            return "<available_skills></available_skills>".to_string();
        }

        let mut lines = vec!["<available_skills>".to_string()];
        for skill in self.list() {
            lines.push("  <skill>".to_string());
            lines.push(format!("    <name>{}</name>", skill.name));
            lines.push(format!("    <description>{}</description>", skill.description));
            lines.push("  </skill>".to_string());
        }
        lines.push("</available_skills>".to_string());
        lines.join("\n")
    }
}

fn load_from_skill_root(
    root: &Path,
    skills: &mut HashMap<String, SkillDefinition>,
    seen: &mut HashSet<String>,
) -> AppResult<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }

        let skill = parse_skill_file(&skill_file)?;
        if seen.insert(skill.name.clone()) {
            skills.insert(skill.name.clone(), skill);
        }
    }

    Ok(())
}

fn parse_skill_file(path: &Path) -> AppResult<SkillDefinition> {
    let content = fs::read_to_string(path)?;
    let (frontmatter_text, body) = split_frontmatter(&content)?;
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(frontmatter_text)?;

    validate_skill_name(&frontmatter.name, path)?;
    if frontmatter.description.trim().is_empty() || frontmatter.description.chars().count() > 1024 {
        return Err(format!("invalid skill description in {}", path.display()).into());
    }

    let dir_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid skill path: {}", path.display()))?;
    if dir_name != frontmatter.name {
        return Err(format!(
            "skill name '{}' does not match directory '{}' in {}",
            frontmatter.name,
            dir_name,
            path.display()
        )
        .into());
    }

    Ok(SkillDefinition {
        name: frontmatter.name,
        description: frontmatter.description,
        license: frontmatter.license,
        compatibility: frontmatter.compatibility,
        metadata: frontmatter.metadata,
        content: body.trim().to_string(),
        path: path.to_path_buf(),
    })
}

fn split_frontmatter(content: &str) -> AppResult<(&str, &str)> {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("SKILL.md must start with YAML frontmatter".into());
    }

    let mut offset = 4usize;
    for line in lines {
        if line.trim() == "---" {
            let frontmatter = &content[4..offset.saturating_sub(1)];
            let body = &content[offset + line.len() + 1..];
            return Ok((frontmatter, body));
        }
        offset += line.len() + 1;
    }

    Err("unterminated YAML frontmatter in SKILL.md".into())
}

fn validate_skill_name(name: &str, path: &Path) -> AppResult<()> {
    let regex = Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$")?;
    if name.is_empty() || name.len() > 64 || !regex.is_match(name) {
        return Err(format!("invalid skill name '{}' in {}", name, path.display()).into());
    }
    Ok(())
}

fn discovery_roots(workspace_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for ancestor in workspace_root.ancestors() {
        roots.push(ancestor.to_path_buf());
        if ancestor.join(".git").exists() {
            break;
        }
    }
    roots
}
