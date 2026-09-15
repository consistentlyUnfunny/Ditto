//! Skill discovery and SKILL.md parsing (F-3.2, F-3.3).
//!
//! Layout is universal across harnesses: `<dir>/<skill-name>/SKILL.md`, YAML
//! frontmatter between `---` fences, markdown below. Only the containing
//! directory differs, and that comes from the provider spec.
//!
//! Two rules that look like edge cases but are not:
//!
//!   - **Nothing is skipped silently.** A directory with no `SKILL.md`, or with
//!     broken frontmatter, is *reported*, not ignored. The dev machine has an
//!     empty `~/.codex/skills/codex-primary-runtime/` that a naive scanner would
//!     drop without a word.
//!   - **Unknown frontmatter keys are preserved verbatim.** Claude Code defines
//!     ~20 optional fields that Codex ignores; they must survive a copy in either
//!     direction (RESEARCH §2).

use crate::spec::SkillsSpec;
use crate::tree::{self, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Skill {
    /// Frontmatter `name`, falling back to the directory name — Claude Code
    /// treats `name` as optional and defaults it exactly this way.
    pub name: String,
    pub description: String,
    /// Every frontmatter key, including `name`/`description` and anything we do
    /// not model. Preserved for round-tripping.
    pub frontmatter: Map,
    pub body: String,
    /// SHA-256 of the normalised file, for drift detection (F-4.2).
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub enum SkillState {
    Ok(Box<Skill>),
    /// A directory that looks like a skill but has no SKILL.md.
    MissingSkillMd,
    /// SKILL.md present but unreadable or with broken frontmatter.
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// The directory name, which is how harnesses actually address a skill.
    pub dir_name: String,
    pub path: PathBuf,
    pub state: SkillState,
}

impl SkillEntry {
    pub fn skill(&self) -> Option<&Skill> {
        match &self.state {
            SkillState::Ok(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_ok(&self) -> bool {
        matches!(self.state, SkillState::Ok(_))
    }
    /// Frontmatter name when parseable, else the directory name. Used for
    /// grouping the same skill across providers.
    pub fn identity(&self) -> &str {
        self.skill().map_or(&self.dir_name, |s| s.name.as_str())
    }
}

/// Scan one skills directory.
///
/// Returns entries in sorted order for stable output. A missing directory yields
/// an empty list, not an error.
pub fn scan_dir(dir: &Path, spec: Option<&SkillsSpec>) -> Vec<SkillEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|path| {
            let dir_name = path.file_name()?.to_string_lossy().to_string();
            if is_excluded(&dir_name, spec) {
                return None;
            }
            Some(read_skill(&path, dir_name))
        })
        .collect()
}

/// Exclusion patterns from the spec (`exclude = [".*"]` by default).
///
/// Only the `.*` prefix form is supported for now — it is the only pattern any
/// real spec needs, and a full glob engine would be unearned complexity. An
/// unsupported pattern excludes nothing rather than silently matching wrongly.
fn is_excluded(dir_name: &str, spec: Option<&SkillsSpec>) -> bool {
    let patterns: &[String] = spec.map_or(&[], |s| s.exclude.as_slice());
    if patterns.is_empty() {
        // Default: hide harness-managed dirs such as ~/.codex/skills/.system/
        return dir_name.starts_with('.');
    }
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => dir_name.starts_with(prefix),
        None => dir_name == p,
    })
}

fn read_skill(path: &Path, dir_name: String) -> SkillEntry {
    let file = path.join("SKILL.md");
    let state = if !file.is_file() {
        SkillState::MissingSkillMd
    } else {
        match std::fs::read_to_string(&file) {
            Err(e) => SkillState::Invalid(e.to_string()),
            Ok(text) => match parse_skill(&text, &dir_name) {
                Ok(skill) => SkillState::Ok(Box::new(skill)),
                Err(reason) => SkillState::Invalid(reason),
            },
        }
    };
    SkillEntry {
        dir_name,
        path: path.to_path_buf(),
        state,
    }
}

/// Split frontmatter from body and parse the YAML.
///
/// A file with no frontmatter is not an error — it is a skill with empty
/// metadata, which Claude Code permits.
pub fn parse_skill(text: &str, dir_name: &str) -> std::result::Result<Skill, String> {
    let (frontmatter_text, body) = split_frontmatter(text);

    let frontmatter = match frontmatter_text {
        None => Map::default(),
        Some(yaml) => match tree::parse(yaml, crate::spec::Format::Yaml) {
            Ok(Value::Map(m)) => m,
            // `---\n---` yields null; treat as empty rather than an error.
            Ok(Value::Null) => Map::default(),
            Ok(other) => {
                return Err(format!(
                    "frontmatter is a {}, expected a mapping",
                    other.type_name()
                ));
            }
            Err(e) => return Err(format!("invalid frontmatter: {e}")),
        },
    };

    let name = frontmatter
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(dir_name)
        .to_string();
    let description = frontmatter
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    Ok(Skill {
        name,
        description,
        frontmatter,
        body: body.to_string(),
        content_hash: hash(text),
    })
}

/// Returns `(frontmatter, body)`. Handles CRLF, because these files are edited on
/// Windows and a `\r` left on the fence would break naive matching.
fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let rest = match text.strip_prefix("---\n") {
        Some(r) => r,
        None => match text.strip_prefix("---\r\n") {
            Some(r) => r,
            None => return (None, text),
        },
    };

    // Find the closing fence at the start of a line.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let frontmatter = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return (Some(frontmatter), body);
        }
        offset += line.len();
    }
    // Unterminated fence: treat the whole file as body rather than guessing.
    (None, text)
}

/// Normalised hash, so drift comparison ignores line endings and trailing
/// whitespace but nothing else (F-4.2).
pub fn hash(text: &str) -> String {
    let normalised: String = text
        .replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    let digest = Sha256::digest(normalised.trim_end().as_bytes());
    // sha2 0.11 returns a `hybrid_array::Array`, which has no `LowerHex` impl,
    // so hex-encode by hand rather than adding a crate for 3 lines.
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "---\nname: karpathy-coder\ndescription: Behavioral guidelines to reduce common LLM coding mistakes.\n---\n\nBehavioral guidelines.\n\n## 1. Think Before Coding\n";

    #[test]
    fn parses_a_real_skill() {
        let s = parse_skill(REAL, "karpathy-coder").unwrap();
        assert_eq!(s.name, "karpathy-coder");
        assert!(s.description.starts_with("Behavioral guidelines"));
        assert!(s.body.contains("## 1. Think Before Coding"));
    }

    /// Real frontmatter from ~/.codex/skills/.system/skill-creator/SKILL.md —
    /// nested maps must survive (RESEARCH §2).
    #[test]
    fn preserves_nested_frontmatter() {
        let text = "---\nname: skill-creator\ndescription: Guide for creating skills.\nmetadata:\n  short-description: Create or update a skill\n---\n\nBody.\n";
        let s = parse_skill(text, "skill-creator").unwrap();
        let metadata = s.frontmatter.get("metadata").unwrap().as_map().unwrap();
        assert_eq!(
            metadata.get("short-description").unwrap().as_str(),
            Some("Create or update a skill")
        );
    }

    /// Claude Code's ~20 optional fields must round-trip even though no other
    /// harness understands them.
    #[test]
    fn preserves_unknown_claude_fields() {
        let text = "---\nname: deploy\ndescription: Deploy it.\ndisable-model-invocation: true\nallowed-tools: Bash(./deploy.sh *)\nargument-hint: \"[env]\"\n---\nBody\n";
        let s = parse_skill(text, "deploy").unwrap();
        assert_eq!(
            s.frontmatter.get("disable-model-invocation"),
            Some(&Value::Bool(true))
        );
        assert!(s.frontmatter.contains_key("allowed-tools"));
        assert!(s.frontmatter.contains_key("argument-hint"));
    }

    #[test]
    fn name_falls_back_to_directory_name() {
        let s = parse_skill("---\ndescription: No name here.\n---\nBody\n", "from-dir").unwrap();
        assert_eq!(s.name, "from-dir");
    }

    #[test]
    fn handles_crlf_frontmatter() {
        let text = "---\r\nname: win\r\ndescription: Windows line endings.\r\n---\r\nBody\r\n";
        let s = parse_skill(text, "win").unwrap();
        assert_eq!(s.name, "win");
        assert_eq!(s.description, "Windows line endings.");
    }

    #[test]
    fn file_without_frontmatter_is_a_skill_with_empty_metadata() {
        let s = parse_skill("Just markdown, no fences.\n", "plain").unwrap();
        assert_eq!(s.name, "plain");
        assert_eq!(s.description, "");
        assert!(s.frontmatter.is_empty());
    }

    #[test]
    fn unterminated_fence_does_not_swallow_the_file() {
        let s = parse_skill("---\nname: broken\nno closing fence\n", "broken").unwrap();
        assert_eq!(s.name, "broken"); // fell back to dir name
        assert!(s.body.contains("no closing fence"));
    }

    /// F-4.2: line endings and trailing whitespace are not drift.
    #[test]
    fn hash_ignores_line_endings_and_trailing_space() {
        assert_eq!(hash("a\nb\n"), hash("a\r\nb\r\n"));
        assert_eq!(hash("a\nb\n"), hash("a   \nb\t\n"));
        assert_eq!(hash("a\nb"), hash("a\nb\n\n\n"));
    }

    /// ...but a real content change is.
    #[test]
    fn hash_detects_a_description_change() {
        let a = parse_skill(REAL, "k").unwrap();
        let b = parse_skill(&REAL.replace("Behavioral", "Different"), "k").unwrap();
        assert_ne!(a.content_hash, b.content_hash);
    }

    #[test]
    fn excludes_dot_directories_by_default() {
        assert!(is_excluded(".system", None));
        assert!(!is_excluded("karpathy-coder", None));
    }

    #[test]
    fn honours_explicit_exclude_patterns() {
        let spec: SkillsSpec = toml::from_str(
            r#"
layout = "dir-with-skill-md"
exclude = [".*", "scratch"]
"#,
        )
        .unwrap();
        assert!(is_excluded(".system", Some(&spec)));
        assert!(is_excluded("scratch", Some(&spec)));
        assert!(!is_excluded("real-skill", Some(&spec)));
    }
}
