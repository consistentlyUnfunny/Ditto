//! Deploy planning and execution (F-5.x).
//!
//! Planning is separated from applying on purpose: a plan can be rendered as a
//! diff, shown, and refused — nothing reaches disk until [`apply`] is called
//! (F-5.3).
//!
//! ## The minimum write set
//!
//! `~/.agents/skills` is read by Codex, OpenCode, Gemini CLI and Copilot CLI;
//! Claude Code refuses it. So "install this skill everywhere" is **two writes,
//! not five** — and the plan says which rule produced each write, because a
//! single write covering four harnesses should not look like three silent
//! skips.

use crate::discover::DetectedProvider;
use crate::editor::Editor;
use crate::emit;
use crate::error::{Error, Result};
use crate::mcp::McpServer;
use crate::secrets::SecretStore;
use crate::spec::{Format, Scope};
use crate::tree::Value;
use crate::write;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConfigWrite {
    /// Providers served by this one write.
    pub providers: Vec<String>,
    pub path: PathBuf,
    pub format: Format,
    /// Pointer to the container holding the server map.
    pub pointer: String,
    pub entry_name: String,
    pub value: Value,
    /// The same entry with credential values masked. The preview renders *this*
    /// one — the emitter resolves secrets to their real values so they can be
    /// written, and printing that into a terminal leaks them (F-7.4).
    pub preview_value: Value,
    /// Harness-specific keys that could not cross (F-5.5).
    pub dropped: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct SkillWrite {
    pub providers: Vec<String>,
    pub source: PathBuf,
    pub dest: PathBuf,
    /// Why this destination was chosen — shown in the preview.
    pub reason: String,
}

#[derive(Debug)]
pub struct ConfigRemoval {
    pub providers: Vec<String>,
    pub path: PathBuf,
    pub format: Format,
    pub pointer: String,
    pub entry_name: String,
}

#[derive(Debug)]
pub struct SkillRemoval {
    /// Providers that will stop seeing the skill once this directory is gone.
    /// A shared directory means removing it affects several at once.
    pub providers: Vec<String>,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum Step {
    Config(ConfigWrite),
    Skill(SkillWrite),
    ConfigRemove(ConfigRemoval),
    SkillRemove(SkillRemoval),
}

/// A target that cannot accept the artifact, and why. Never a silent skip.
#[derive(Debug)]
pub struct Refusal {
    pub provider: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub refusals: Vec<Refusal>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Every provider this plan will actually reach.
    pub fn providers_served(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self
            .steps
            .iter()
            .flat_map(|s| match s {
                Step::Config(c) => c.providers.iter(),
                Step::Skill(s) => s.providers.iter(),
                Step::ConfigRemove(c) => c.providers.iter(),
                Step::SkillRemove(s) => s.providers.iter(),
            })
            .map(String::as_str)
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }
}

// ---------------------------------------------------------------------------
// Planning — MCP
// ---------------------------------------------------------------------------

/// Plan deploying one server to each named provider.
///
/// Each harness keeps MCP servers in its own file, so there is no sharing to
/// exploit here — one write per target.
pub fn mcp_deploy(
    providers: &[DetectedProvider<'_>],
    targets: &[String],
    server: &McpServer,
    scope: Scope,
    secrets: &dyn SecretStore,
) -> Plan {
    let mut plan = Plan::default();

    for target in targets {
        let Some(provider) = providers.iter().find(|p| p.id() == target) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not a known provider".to_string(),
            });
            continue;
        };
        if !provider.installed {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not installed on this machine".to_string(),
            });
            continue;
        }
        let Some(mcp_spec) = provider.spec.mcp.as_ref().filter(|m| m.supported) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "does not support MCP servers".to_string(),
            });
            continue;
        };

        let emitted = match emit::server(mcp_spec, target, server, secrets) {
            Ok(e) => e,
            Err(e) => {
                plan.refusals.push(Refusal {
                    provider: target.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
        };

        let Some(location) = mcp_spec.locations.iter().find(|l| l.scope == scope) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: format!("has no {scope:?}-scope location for MCP servers"),
            });
            continue;
        };
        let Some(file) = provider.file(&location.file).and_then(|f| f.write_target()) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "config file location could not be resolved".to_string(),
            });
            continue;
        };
        // NF-1.6: never write into a file we could not parse.
        if let crate::tree::FileState::Invalid(reason) = &file.state {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: format!("{} could not be parsed: {reason}", file.path.display()),
            });
            continue;
        }

        let preview_value = emit::redacted_copy(mcp_spec, &emitted.value);
        plan.steps.push(Step::Config(ConfigWrite {
            providers: vec![target.clone()],
            path: file.path.clone(),
            format: file.format,
            pointer: location.pointer.clone(),
            entry_name: server.name.clone(),
            value: emitted.value,
            preview_value,
            dropped: emitted.dropped,
            warnings: emitted.warnings,
        }));
    }

    plan
}

// ---------------------------------------------------------------------------
// Planning — skills, with the minimum write set (F-5.2)
// ---------------------------------------------------------------------------

pub fn skill_deploy(
    providers: &[DetectedProvider<'_>],
    targets: &[String],
    source: &Path,
    scope: Scope,
) -> Plan {
    let mut plan = Plan::default();

    let skill_name = match source.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => {
            plan.refusals.push(Refusal {
                provider: "*".to_string(),
                reason: format!("{} is not a skill directory", source.display()),
            });
            return plan;
        }
    };

    // Which targets are usable at all.
    let mut pending: Vec<&DetectedProvider<'_>> = Vec::new();
    for target in targets {
        match providers.iter().find(|p| p.id() == target) {
            None => plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not a known provider".to_string(),
            }),
            Some(p) if !p.installed => plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not installed on this machine".to_string(),
            }),
            Some(p) if !p.spec.supports_skills() => plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "does not support skills".to_string(),
            }),
            Some(p) => pending.push(p),
        }
    }

    // Step 1: collapse onto shared directories that serve two or more targets.
    //
    // Greedy by coverage. With one shared path in play this is exact; if several
    // ever overlap, greedy set-cover is a fine approximation for a set of five.
    loop {
        let mut coverage: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
        for provider in &pending {
            // Uses the *resolved* dirs, not the spec templates: several templates
            // end in "skills", so matching by name would conflate
            // `~/.agents/skills` with `~/.codex/skills`.
            for dir in &provider.skill_dirs {
                if dir.scope != scope || !dir.shared || dir.deprecated {
                    continue;
                }
                coverage
                    .entry(dir.path.clone())
                    .or_default()
                    .push(provider.id().to_string());
            }
        }

        let Some((path, covered)) = coverage.into_iter().max_by_key(|(_, v)| v.len()) else {
            break;
        };
        if covered.len() < 2 {
            break;
        }

        plan.steps.push(Step::Skill(SkillWrite {
            providers: covered.clone(),
            source: source.to_path_buf(),
            dest: path.join(&skill_name),
            reason: format!("shared directory read by {}", covered.join(", ")),
        }));
        pending.retain(|p| !covered.iter().any(|id| id == p.id()));
    }

    // Step 2: everyone left gets their own canonical directory.
    for provider in pending {
        let canonical = provider
            .skill_dirs
            .iter()
            .find(|d| d.scope == scope && d.canonical && !d.deprecated);

        match canonical {
            Some(dir) => plan.steps.push(Step::Skill(SkillWrite {
                providers: vec![provider.id().to_string()],
                source: source.to_path_buf(),
                dest: dir.path.join(&skill_name),
                reason: if dir.shared {
                    // Codex's only non-deprecated user dir IS the shared one, so a
                    // Codex-only deploy is visible to other harnesses. Say so
                    // rather than pretend the write was private.
                    format!(
                        "{}'s canonical directory, which other harnesses also read",
                        provider.id()
                    )
                } else {
                    format!("{}'s own directory", provider.id())
                },
            })),
            None => plan.refusals.push(Refusal {
                provider: provider.id().to_string(),
                reason: format!("has no writable {scope:?}-scope skill directory"),
            }),
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// Planning — removal (F-5.7)
// ---------------------------------------------------------------------------

/// Plan removing an MCP server from each named provider.
pub fn mcp_remove(
    providers: &[DetectedProvider<'_>],
    targets: &[String],
    name: &str,
    scope: Scope,
) -> Plan {
    let mut plan = Plan::default();

    for target in targets {
        let Some(provider) = providers.iter().find(|p| p.id() == target) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not a known provider".to_string(),
            });
            continue;
        };
        let Some(mcp_spec) = provider.spec.mcp.as_ref().filter(|m| m.supported) else {
            continue;
        };
        for location in mcp_spec.locations.iter().filter(|l| l.scope == scope) {
            let Some(file) = provider.file(&location.file).and_then(|f| f.primary()) else {
                continue;
            };
            let Some(doc) = file.state.value() else { continue };

            // Only plan a removal where the entry actually exists, so the preview
            // never shows a no-op.
            let present = doc
                .at_pointer(&location.pointer)
                .ok()
                .flatten()
                .and_then(Value::as_map)
                .is_some_and(|m| m.contains_key(name));
            if !present {
                continue;
            }

            plan.steps.push(Step::ConfigRemove(ConfigRemoval {
                providers: vec![target.clone()],
                path: file.path.clone(),
                format: file.format,
                pointer: location.pointer.clone(),
                entry_name: name.to_string(),
            }));
        }
    }

    if plan.steps.is_empty() && plan.refusals.is_empty() {
        plan.refusals.push(Refusal {
            provider: targets.join(", "),
            reason: format!("no MCP server named '{name}' is installed there"),
        });
    }
    plan
}

/// Plan removing a skill directory.
///
/// A shared directory serves several harnesses, so deleting it takes the skill
/// away from all of them. The plan names every affected provider, because that
/// belongs in the confirmation prompt rather than in a surprise afterwards.
pub fn skill_remove(
    providers: &[DetectedProvider<'_>],
    targets: &[String],
    name: &str,
    scope: Scope,
) -> Plan {
    let mut plan = Plan::default();
    let mut seen: Vec<PathBuf> = Vec::new();

    for target in targets {
        let Some(provider) = providers.iter().find(|p| p.id() == target) else {
            plan.refusals.push(Refusal {
                provider: target.clone(),
                reason: "not a known provider".to_string(),
            });
            continue;
        };
        for dir in provider.skill_dirs.iter().filter(|d| d.scope == scope && d.exists) {
            let path = dir.path.join(name);
            if !path.is_dir() || seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());

            // Everyone who reads this directory loses the skill.
            let mut affected: Vec<String> = providers
                .iter()
                .filter(|p| p.installed)
                .filter(|p| p.skill_dirs.iter().any(|d| d.scope == scope && d.path == dir.path))
                .map(|p| p.id().to_string())
                .collect();
            affected.sort();

            plan.steps.push(Step::SkillRemove(SkillRemoval { providers: affected, path }));
        }
    }

    if plan.steps.is_empty() && plan.refusals.is_empty() {
        plan.refusals.push(Refusal {
            provider: targets.join(", "),
            reason: format!("no skill named '{name}' is installed there"),
        });
    }
    plan
}

// ---------------------------------------------------------------------------
// Preview (F-5.3)
// ---------------------------------------------------------------------------

/// Render a plan as text, including a unified diff of every config change.
///
/// Computes the new file contents *without writing them*, which means the preview
/// is the real thing rather than a description of it.
pub fn preview(plan: &Plan) -> Result<String> {
    use std::fmt::Write as _;
    let mut out = String::new();

    for step in &plan.steps {
        match step {
            Step::Config(change) => {
                let _ = writeln!(
                    out,
                    "{} <- {} ({})",
                    change.path.display(),
                    change.entry_name,
                    change.providers.join(", ")
                );
                let (before, after) = render_change(change, Redact::Yes)?;
                out.push_str(&unified_diff(&before, &after));

                if !change.dropped.is_empty() {
                    let _ = writeln!(
                        out,
                        "  ! dropped (not supported by {}): {}",
                        change.providers.join(", "),
                        change.dropped.join(", ")
                    );
                }
                for warning in &change.warnings {
                    let _ = writeln!(out, "  ! {warning}");
                }
                out.push('\n');
            }
            Step::ConfigRemove(removal) => {
                let _ = writeln!(
                    out,
                    "{} -- remove {} ({})",
                    removal.path.display(),
                    removal.entry_name,
                    removal.providers.join(", ")
                );
                let before = std::fs::read_to_string(&removal.path).unwrap_or_default();
                let mut editor = Editor::parse(&before, removal.format)?;
                editor.remove(&removal.pointer, &removal.entry_name)?;
                out.push_str(&unified_diff(&before, &editor.render()));
                out.push('\n');
            }
            Step::SkillRemove(removal) => {
                let _ = writeln!(out, "{}", removal.path.display());
                let _ = writeln!(out, "  - DELETE this directory and everything in it");
                if removal.providers.len() > 1 {
                    let _ = writeln!(
                        out,
                        "  ! shared directory — this removes the skill from all of: {}",
                        removal.providers.join(", ")
                    );
                } else {
                    let _ = writeln!(out, "  affects: {}", removal.providers.join(", "));
                }
                out.push('\n');
            }
            Step::Skill(copy) => {
                let _ = writeln!(out, "{}", copy.dest.display());
                let _ = writeln!(out, "  + copy from {}", copy.source.display());
                let _ = writeln!(out, "  serves: {}", copy.providers.join(", "));
                let _ = writeln!(out, "  reason: {}", copy.reason);
                out.push('\n');
            }
        }
    }

    for refusal in &plan.refusals {
        let _ = writeln!(out, "REFUSED {}: {}", refusal.provider, refusal.reason);
    }

    Ok(out)
}

/// Whether to render with credentials masked. Display always masks; the write
/// path never does.
#[derive(Clone, Copy, PartialEq)]
enum Redact {
    Yes,
    No,
}

/// Produce the before/after text for one config change.
fn render_change(change: &ConfigWrite, redact: Redact) -> Result<(String, String)> {
    let before = std::fs::read_to_string(&change.path).unwrap_or_default();
    let mut editor = Editor::parse(&before, change.format)?;
    let value = match redact {
        Redact::Yes => &change.preview_value,
        Redact::No => &change.value,
    };
    // `merge`, not `upsert`: an update must not delete fields it did not set
    // (PROVIDER-SPEC §6 rule 3).
    editor.merge(&change.pointer, &change.entry_name, value)?;
    Ok((before, editor.render()))
}

fn unified_diff(before: &str, after: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    for group in diff.grouped_ops(3) {
        for op in group {
            for change in diff.iter_changes(&op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str("  ");
                out.push_str(sign);
                out.push_str(change.value());
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Outcome {
    pub target: PathBuf,
    pub providers: Vec<String>,
    pub result: std::result::Result<String, String>,
}

/// Execute a plan.
///
/// NF-2.4: a failure part-way does not abort the rest, and every step reports
/// its own result — so the user always learns exactly what was and was not
/// written.
pub fn apply(plan: &Plan) -> Vec<Outcome> {
    let mut outcomes = Vec::new();

    for step in &plan.steps {
        match step {
            Step::Config(change) => {
                let result = (|| -> Result<String> {
                    let (_, after) = render_change(change, Redact::No)?;
                    let report = write::save(&change.path, &after)?;
                    Ok(if report.changed {
                        match report.backup {
                            Some(b) => format!("written (backup: {})", b.display()),
                            None => "written".to_string(),
                        }
                    } else {
                        "already up to date".to_string()
                    })
                })();
                outcomes.push(Outcome {
                    target: change.path.clone(),
                    providers: change.providers.clone(),
                    result: result.map_err(|e| e.to_string()),
                });
            }
            Step::ConfigRemove(removal) => {
                let result = (|| -> Result<String> {
                    let before = std::fs::read_to_string(&removal.path).unwrap_or_default();
                    let mut editor = Editor::parse(&before, removal.format)?;
                    if !editor.remove(&removal.pointer, &removal.entry_name)? {
                        return Ok("not present".to_string());
                    }
                    let report = write::save(&removal.path, &editor.render())?;
                    Ok(match report.backup {
                        Some(b) => format!("removed (backup: {})", b.display()),
                        None => "removed".to_string(),
                    })
                })();
                outcomes.push(Outcome {
                    target: removal.path.clone(),
                    providers: removal.providers.clone(),
                    result: result.map_err(|e| e.to_string()),
                });
            }
            Step::SkillRemove(removal) => {
                // Deleting a directory is the one irreversible action here — a
                // backup of a whole skill tree is out of scope, so the preview
                // and confirmation carry the weight.
                let result = std::fs::remove_dir_all(&removal.path)
                    .map(|_| "deleted".to_string())
                    .map_err(|e| e.to_string());
                outcomes.push(Outcome {
                    target: removal.path.clone(),
                    providers: removal.providers.clone(),
                    result,
                });
            }
            Step::Skill(copy) => {
                let result = copy_dir(&copy.source, &copy.dest)
                    .map(|n| format!("copied {n} file(s)"))
                    .map_err(|e| e.to_string());
                outcomes.push(Outcome {
                    target: copy.dest.clone(),
                    providers: copy.providers.clone(),
                    result,
                });
            }
        }
    }

    outcomes
}

/// Recursive copy. Returns the number of files written.
///
/// Copy, never symlink (F-5.6) — Windows symlinks need Developer Mode or
/// elevation, and junctions do not work for files.
fn copy_dir(source: &Path, dest: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|e| Error::NoSuchDirectory {
            what: if e.path().is_some() { "skill source" } else { "skill" },
        })?;
        let relative = entry.path().strip_prefix(source).unwrap_or(entry.path());
        let target = dest.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(Error::reading(&target))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(Error::reading(parent))?;
            }
            std::fs::copy(entry.path(), &target).map_err(Error::reading(&target))?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_marks_added_lines() {
        let d = unified_diff("a\nb\n", "a\nb\nc\n");
        assert!(d.contains("+c"));
        assert!(!d.contains("-a"));
    }

    #[test]
    fn empty_plan_reports_empty() {
        assert!(Plan::default().is_empty());
        assert!(Plan::default().providers_served().is_empty());
    }
}
