//! Cross-provider inventory and drift detection (F-3.1, F-4.x).
//!
//! ## Why locations are deduplicated by path
//!
//! `~/.agents/skills` is read by four of the five harnesses. A naive scan walks
//! each provider's directory list and reports the same physical skill four times,
//! which reads as "installed in four places" when it is one file. So locations
//! are keyed by **path**, and each carries the list of providers that read it.
//!
//! This is also what makes the minimum-write-set planner possible later (F-5.2):
//! once you know one path serves four providers, "deploy everywhere" is obviously
//! two writes rather than five.

use crate::discover::DetectedProvider;
use crate::mcp::{self, McpServer};
use crate::skills::{self, SkillEntry};
use crate::spec::Scope;
use crate::tree::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One skill directory on disk, plus every provider that reads it.
#[derive(Debug)]
pub struct SkillLocation {
    pub path: PathBuf,
    /// Provider ids that read this path, sorted. More than one means a shared dir.
    pub providers: Vec<String>,
    pub entry: SkillEntry,
    pub shared: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftStatus {
    /// Present in exactly one place.
    Unique,
    /// Present in several, all byte-equal after normalisation.
    Identical,
    /// Present in several, with differing content.
    Drifted,
    /// At least one copy could not be read or parsed.
    Problem,
}

impl DriftStatus {
    pub fn label(self) -> &'static str {
        match self {
            DriftStatus::Unique => "unique",
            DriftStatus::Identical => "identical",
            DriftStatus::Drifted => "DRIFTED",
            DriftStatus::Problem => "PROBLEM",
        }
    }
}

#[derive(Debug)]
pub struct SkillGroup {
    /// Frontmatter `name` where available, else the directory name.
    pub identity: String,
    pub locations: Vec<SkillLocation>,
    pub status: DriftStatus,
}

impl SkillGroup {
    /// Every provider that can see this skill, across all its locations.
    pub fn providers(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self
            .locations
            .iter()
            .flat_map(|l| l.providers.iter().map(String::as_str))
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Distinct content hashes — more than one means real divergence.
    pub fn distinct_hashes(&self) -> Vec<&str> {
        let mut hashes: Vec<&str> = self
            .locations
            .iter()
            .filter_map(|l| l.entry.skill().map(|s| s.content_hash.as_str()))
            .collect();
        hashes.sort_unstable();
        hashes.dedup();
        hashes
    }
}

/// Build the skill inventory for a scope.
pub fn skills(providers: &[DetectedProvider<'_>], scope: Scope) -> Vec<SkillGroup> {
    // path -> (providers reading it, flags)
    let mut dirs: BTreeMap<PathBuf, (Vec<String>, bool, bool)> = BTreeMap::new();
    for provider in providers {
        if !provider.installed || !provider.spec.supports_skills() {
            continue;
        }
        for dir in provider.existing_skill_dirs(scope) {
            let slot = dirs
                .entry(dir.path.clone())
                .or_insert_with(|| (Vec::new(), dir.shared, dir.deprecated));
            slot.0.push(provider.id().to_string());
            // A dir is shared in practice if any spec marks it so.
            slot.1 |= dir.shared;
            slot.2 |= dir.deprecated;
        }
    }

    // identity -> locations
    let mut groups: BTreeMap<String, Vec<SkillLocation>> = BTreeMap::new();
    for (path, (mut provider_ids, shared, deprecated)) in dirs {
        provider_ids.sort();
        provider_ids.dedup();

        // Pull the exclude patterns from any provider reading this dir; they are
        // identical across built-in specs and the spec is the authority.
        let skills_spec = providers
            .iter()
            .find(|p| provider_ids.iter().any(|id| id == p.id()))
            .and_then(|p| p.spec.skills.as_ref());

        for entry in skills::scan_dir(&path, skills_spec) {
            groups
                .entry(entry.identity().to_string())
                .or_default()
                .push(SkillLocation {
                    path: entry.path.clone(),
                    providers: provider_ids.clone(),
                    entry,
                    shared,
                    deprecated,
                });
        }
    }

    groups
        .into_iter()
        .map(|(identity, locations)| {
            let status = classify(&locations);
            SkillGroup {
                identity,
                locations,
                status,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// MCP servers
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct McpLocation {
    pub provider: String,
    pub file: PathBuf,
    pub scope: Scope,
    /// For a project-namespaced entry, which project it belongs to.
    pub project: Option<String>,
    pub server: McpServer,
}

/// Collect MCP servers from every detected provider.
///
/// ## Enumerating project-namespaced entries
///
/// Claude Code stores project-local servers at
/// `/projects/{project_path}/mcpServers` inside the *user* file. When no
/// specific project is requested we cannot substitute `{project_path}`, so
/// instead we resolve the pointer prefix up to the placeholder and enumerate
/// every key found there. That turns "I can't fill this in" into "show me all of
/// them", which is what someone auditing their machine actually wants.
pub fn mcp_servers(providers: &[DetectedProvider<'_>], scope: Scope) -> Vec<McpLocation> {
    let mut out = Vec::new();

    for provider in providers {
        if !provider.installed {
            continue;
        }
        let Some(mcp_spec) = &provider.spec.mcp else {
            continue;
        };
        if !mcp_spec.supported {
            continue;
        }

        for location in &mcp_spec.locations {
            // A user-scope listing also surfaces project-namespaced entries,
            // since they live in the same file and are easy to forget about.
            let wanted = location.scope == scope
                || (scope == Scope::User && location.scope == Scope::Local);
            if !wanted {
                continue;
            }
            let Some(file) = provider.file(&location.file) else {
                continue;
            };
            let Some(config) = file.primary() else { continue };
            let Some(doc) = config.state.value() else {
                continue;
            };

            for (pointer, project) in resolve_pointer(doc, &location.pointer) {
                let Ok(Some(servers)) = doc.at_pointer(&pointer) else {
                    continue;
                };
                for server in mcp::read_servers(mcp_spec, provider.id(), servers) {
                    out.push(McpLocation {
                        provider: provider.id().to_string(),
                        file: config.path.clone(),
                        scope: location.scope,
                        project: project.clone(),
                        server,
                    });
                }
            }
        }
    }
    out
}

/// Expand a pointer template into concrete pointers.
///
/// Without a placeholder this is the identity. With one, the prefix is resolved
/// and every key under it becomes a candidate.
fn resolve_pointer(doc: &Value, template: &str) -> Vec<(String, Option<String>)> {
    let Some(start) = template.find('{') else {
        return vec![(template.to_string(), None)];
    };
    let Some(end) = template[start..].find('}').map(|i| start + i) else {
        return vec![(template.to_string(), None)];
    };

    // "/projects/{project_path}/mcpServers" -> prefix "/projects", suffix "/mcpServers"
    let prefix = template[..start].trim_end_matches('/');
    let suffix = &template[end + 1..];

    let Ok(Some(container)) = doc.at_pointer(prefix) else {
        return Vec::new();
    };
    let Some(map) = container.as_map() else {
        return Vec::new();
    };

    map.keys()
        .map(|key| {
            let escaped = crate::pointer::escape_token(key);
            (
                format!("{prefix}/{escaped}{suffix}"),
                Some(key.to_string()),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Comparing copies
// ---------------------------------------------------------------------------

/// The copy a deploy uses as its source when the caller does not choose one.
///
/// Currently the first readable copy in path order — which is arbitrary, and is
/// exactly why the UI has to *show* the answer rather than leave it implicit.
/// Both the planner and the detail view call this, so the interface can never
/// name a different source than the one that will actually be used.
///
/// T-5.14 replaces the implicit choice with an explicit one; this stays as the
/// default for callers that don't care.
pub fn default_source(group: &SkillGroup) -> Option<&SkillLocation> {
    group.locations.iter().find(|l| l.entry.is_ok())
}

/// The copy belonging to a named provider, for `--from <provider>`.
///
/// A harness often reads several directories, so "OpenCode's version" is
/// ambiguous when they disagree — OpenCode sees both its own
/// `~/.config/opencode/skills` copy and whatever sits in the shared
/// `~/.agents/skills`. Path order picked the shared one, which is the opposite
/// of what someone means by "use OpenCode's copy".
///
/// Preference, in order:
///   1. a directory only this harness reads, still current
///   2. a shared directory
///   3. a deprecated location, as a last resort
///
/// Codex lands on the shared directory by that rule, correctly: its only
/// private directory is the deprecated one.
pub fn source_for<'a>(group: &'a SkillGroup, provider: &str) -> Option<&'a SkillLocation> {
    let mine = |l: &&SkillLocation| {
        l.entry.is_ok() && l.providers.iter().any(|p| p == provider)
    };
    let rank = |l: &SkillLocation| match (l.shared, l.deprecated) {
        (false, false) => 0,
        (true, false) => 1,
        _ => 2,
    };
    group.locations.iter().filter(mine).min_by_key(|l| rank(l))
}

/// The distinct versions of a skill, grouped by content.
///
/// Four copies holding two versions is a choice between *two* things, not four —
/// which is how the resolve flow has to present it.
pub fn versions(group: &SkillGroup) -> Vec<Vec<&SkillLocation>> {
    let mut groups: Vec<(String, Vec<&SkillLocation>)> = Vec::new();
    for location in group.locations.iter().filter(|l| l.entry.is_ok()) {
        let hash = location
            .entry
            .skill()
            .map(|s| s.content_hash.clone())
            .unwrap_or_default();
        match groups.iter_mut().find(|(h, _)| *h == hash) {
            Some((_, members)) => members.push(location),
            None => groups.push((hash, vec![location])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// One side of a comparison, with enough identity to label it.
///
/// A comparison is between two *versions*, so the reader needs to know whose
/// each one is — "codex, claude" tells them something a file path does not.
#[derive(Debug)]
pub struct CopySide {
    pub path: PathBuf,
    pub providers: Vec<String>,
    pub hash: String,
}

/// A line-by-line comparison of two copies of the same skill.
#[derive(Debug)]
pub struct CopyComparison {
    pub left: CopySide,
    pub right: CopySide,
    pub lines: Vec<crate::diff::DiffLine>,
}

/// Diff the first two readable copies of a skill, so "these differ" can be
/// followed by *how*.
///
/// Compares the whole `SKILL.md`, not just the frontmatter: a change to the body
/// is every bit as real as a change to the description, and the hashes that
/// drive drift detection cover the whole file.
///
/// Returns `None` when there is nothing to compare — fewer than two readable
/// copies.
pub fn compare_copies(group: &SkillGroup) -> Option<CopyComparison> {
    let readable: Vec<&SkillLocation> =
        group.locations.iter().filter(|l| l.entry.is_ok()).collect();
    let left = readable.first()?;

    // Compare against the first copy that actually *differs*, not simply the
    // next one in path order. With four copies where three agree, taking
    // "the first two" produces an empty diff and the drift goes unexplained —
    // which is exactly what happened the first time this shipped.
    let left_hash = left.entry.skill().map(|s| &s.content_hash);
    let right = readable
        .iter()
        .skip(1)
        .find(|l| l.entry.skill().map(|s| &s.content_hash) != left_hash)
        .or_else(|| readable.get(1))?;

    let read = |location: &SkillLocation| {
        std::fs::read_to_string(location.path.join("SKILL.md")).unwrap_or_default()
    };

    let side = |l: &SkillLocation| CopySide {
        path: l.path.clone(),
        providers: l.providers.clone(),
        hash: l
            .entry
            .skill()
            .map(|s| s.content_hash.chars().take(8).collect())
            .unwrap_or_default(),
    };

    Some(CopyComparison {
        left: side(left),
        right: side(right),
        // Generous context: these files are short, and seeing an edit in its
        // surroundings is the point.
        lines: crate::diff::lines(&read(left), &read(right), 4),
    })
}

fn classify(locations: &[SkillLocation]) -> DriftStatus {
    if locations.iter().any(|l| !l.entry.is_ok()) {
        return DriftStatus::Problem;
    }
    if locations.len() == 1 {
        return DriftStatus::Unique;
    }
    let first = locations[0].entry.skill().map(|s| &s.content_hash);
    let all_same = locations
        .iter()
        .all(|l| l.entry.skill().map(|s| &s.content_hash) == first);
    if all_same {
        DriftStatus::Identical
    } else {
        DriftStatus::Drifted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SkillEntry, SkillState, parse_skill};

    fn location(path: &str, providers: &[&str], text: &str) -> SkillLocation {
        SkillLocation {
            path: PathBuf::from(path),
            providers: providers.iter().map(|s| s.to_string()).collect(),
            entry: SkillEntry {
                dir_name: "s".into(),
                path: PathBuf::from(path),
                state: SkillState::Ok(Box::new(parse_skill(text, "s").unwrap())),
            },
            shared: false,
            deprecated: false,
        }
    }

    const A: &str = "---\nname: s\ndescription: One.\n---\nBody\n";
    const B: &str = "---\nname: s\ndescription: Two, different.\n---\nBody\n";

    #[test]
    fn single_copy_is_unique() {
        assert_eq!(classify(&[location("a", &["codex"], A)]), DriftStatus::Unique);
    }

    #[test]
    fn matching_copies_are_identical() {
        let locs = vec![location("a", &["codex"], A), location("b", &["opencode"], A)];
        assert_eq!(classify(&locs), DriftStatus::Identical);
    }

    /// The headline acceptance test (F-4.5), in miniature: same name, different
    /// description.
    #[test]
    fn differing_descriptions_are_drift() {
        let locs = vec![location("a", &["codex"], A), location("b", &["opencode"], B)];
        assert_eq!(classify(&locs), DriftStatus::Drifted);
        let group = SkillGroup {
            identity: "s".into(),
            locations: locs,
            status: DriftStatus::Drifted,
        };
        assert_eq!(group.distinct_hashes().len(), 2);
        assert_eq!(group.providers(), vec!["codex", "opencode"]);
    }

    /// Line endings alone must not register as drift (F-4.2).
    #[test]
    fn line_ending_differences_are_not_drift() {
        let locs = vec![
            location("a", &["codex"], A),
            location("b", &["opencode"], &A.replace('\n', "\r\n")),
        ];
        assert_eq!(classify(&locs), DriftStatus::Identical);
    }

    #[test]
    fn pointer_without_placeholder_is_unchanged() {
        let doc = crate::tree::parse("{}", crate::spec::Format::Jsonc).unwrap();
        let resolved = resolve_pointer(&doc, "/mcpServers");
        assert_eq!(resolved, vec![("/mcpServers".to_string(), None)]);
    }

    /// The real shape of ~/.claude.json: enumerate every project key, escaping
    /// the Windows path in each generated pointer.
    #[test]
    fn placeholder_pointer_enumerates_projects() {
        let doc = crate::tree::parse(
            r#"{"projects": {"C:/Users/User": {}, "D:/Projects/demo": {}}}"#,
            crate::spec::Format::Jsonc,
        )
        .unwrap();
        let resolved = resolve_pointer(&doc, "/projects/{project_path}/mcpServers");
        assert_eq!(resolved.len(), 2);
        assert_eq!(
            resolved[0],
            (
                "/projects/C:~1Users~1User/mcpServers".to_string(),
                Some("C:/Users/User".to_string())
            )
        );
        // And the generated pointers must actually resolve.
        for (pointer, _) in &resolved {
            assert!(doc.at_pointer(pointer).is_ok());
        }
    }

    #[test]
    fn placeholder_pointer_on_missing_container_yields_nothing() {
        let doc = crate::tree::parse("{}", crate::spec::Format::Jsonc).unwrap();
        assert!(resolve_pointer(&doc, "/projects/{project_path}/mcpServers").is_empty());
    }

    #[test]
    fn unreadable_copy_is_a_problem() {
        let mut locs = vec![location("a", &["codex"], A)];
        locs.push(SkillLocation {
            path: PathBuf::from("b"),
            providers: vec!["opencode".into()],
            entry: SkillEntry {
                dir_name: "s".into(),
                path: PathBuf::from("b"),
                state: SkillState::MissingSkillMd,
            },
            shared: false,
            deprecated: false,
        });
        assert_eq!(classify(&locs), DriftStatus::Problem);
    }
}
