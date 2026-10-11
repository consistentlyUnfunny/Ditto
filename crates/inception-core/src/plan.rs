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
use crate::diff::DiffLine;
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
    /// True when `dest` already holds a copy whose content differs. The preview
    /// shows the diff, because this write destroys what is there.
    pub replaces_divergent: bool,
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

    // Which targets are usable at all. `pending` is drained as writes are
    // assigned; `valid` keeps the full list, because convergence below has to
    // consider a harness even when it needs no write of its own.
    let mut valid: Vec<String> = Vec::new();
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
            Some(p) => {
                valid.push(p.id().to_string());
                pending.push(p);
            }
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
            replaces_divergent: differs(source, &path.join(&skill_name)),
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
                replaces_divergent: differs(source, &dir.path.join(&skill_name)),
            })),
            None => plan.refusals.push(Refusal {
                provider: provider.id().to_string(),
                reason: format!("has no writable {scope:?}-scope skill directory"),
            }),
        }
    }

    // --- converge, don't just add ------------------------------------------
    //
    // A harness usually reads several skill directories. Writing the source into
    // one of them while a *different*, divergent copy sits in another leaves that
    // harness seeing two versions of the same skill — which is worse than the
    // drift we set out to fix. So every other copy a target can see is
    // overwritten too, and the preview shows each as a replacement.
    let destinations: Vec<PathBuf> = plan
        .steps
        .iter()
        .filter_map(|s| match s {
            Step::Skill(w) => Some(w.dest.clone()),
            _ => None,
        })
        .collect();

    let mut extra: Vec<SkillWrite> = Vec::new();
    for target in &valid {
        let Some(provider) = providers.iter().find(|p| p.id() == target) else {
            continue;
        };
        for dir in provider.skill_dirs.iter().filter(|d| d.scope == scope && d.exists) {
            let existing = dir.path.join(&skill_name);
            if !existing.is_dir()
                || existing == source
                || destinations.contains(&existing)
                || extra.iter().any(|w| w.dest == existing)
            {
                continue;
            }
            if !differs(source, &existing) {
                continue; // already agrees; nothing to converge
            }
            extra.push(SkillWrite {
                providers: vec![target.clone()],
                source: source.to_path_buf(),
                dest: existing,
                reason: format!(
                    "{target} reads this directory too; a copy here that differs would shadow it"
                ),
                replaces_divergent: true,
            });
        }
    }
    plan.steps.extend(extra.into_iter().map(Step::Skill));

    // A write that cannot change anything is noise in the preview and a lie in
    // the count. Two kinds: copying a directory onto itself, and copying onto a
    // destination that already holds exactly this content. Dropping both is what
    // makes "nothing to do" mean something — and what makes a second deploy of
    // the same thing converge instead of looking like fresh work.
    plan.steps.retain(|step| match step {
        Step::Skill(w) => w.dest != source && !identical(source, &w.dest),
        _ => true,
    });

    plan
}

// ---------------------------------------------------------------------------
// Import — adopting a copy into the library (Q-7)
// ---------------------------------------------------------------------------

/// Plan the copy of `source` into the library.
///
/// An import is an ordinary [`Step::Skill`] write, for the same reason a scope
/// transfer is an ordinary deploy (F-5.9): it then shares the preview, the
/// replacement diff, the confirmation and `apply` with every other write, and
/// cannot quietly disagree with them. The only thing that distinguishes it is
/// that no harness reads the destination, so `providers` is empty.
///
/// Deploying never calls this. Adopting a skill is a decision, not a side
/// effect of sending it somewhere — see [`crate::settings`].
pub fn skill_import(source: &Path, library: &Path) -> Plan {
    let mut plan = Plan::default();

    let Some(name) = source.file_name() else {
        plan.refusals.push(Refusal {
            provider: "library".to_string(),
            reason: format!("{} is not a skill directory", source.display()),
        });
        return plan;
    };

    let dest = library.join(name);

    // Importing the library's own copy back into itself is the one case that
    // cannot do anything. Say so rather than reporting a no-op write.
    if dest == source {
        plan.refusals.push(Refusal {
            provider: "library".to_string(),
            reason: "that copy is already the library's".to_string(),
        });
        return plan;
    }

    if identical(source, &dest) {
        plan.refusals.push(Refusal {
            provider: "library".to_string(),
            reason: "the library already holds exactly this content".to_string(),
        });
        return plan;
    }

    plan.steps.push(Step::Skill(SkillWrite {
        providers: Vec::new(),
        source: source.to_path_buf(),
        dest,
        reason: "your library — the copy a deploy uses when you don't name one".to_string(),
        replaces_divergent: differs(source, &library.join(name)),
    }));
    plan
}

/// Whether `dest` already holds byte-for-byte what `source` would put there.
///
/// Compares the whole tree, not just `SKILL.md`: a skill is a directory, and a
/// changed helper script is a real change even when the frontmatter agrees.
fn identical(source: &Path, dest: &Path) -> bool {
    if !dest.is_dir() {
        return false;
    }
    let files = |root: &Path| -> Option<BTreeMap<PathBuf, Vec<u8>>> {
        let mut out = BTreeMap::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.ok()?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry.path().strip_prefix(root).ok()?.to_path_buf();
            out.insert(relative, std::fs::read(entry.path()).ok()?);
        }
        Some(out)
    };
    match (files(source), files(dest)) {
        (Some(a), Some(b)) => a == b,
        // Unreadable either side: assume a write is needed and let it report.
        _ => false,
    }
}

/// Whether `dest` already holds a skill whose content differs from `source`.
///
/// Compared on the same normalised hash drift detection uses, so the two can
/// never disagree about whether something has changed.
fn differs(source: &Path, dest: &Path) -> bool {
    let read = |dir: &Path| std::fs::read_to_string(dir.join("SKILL.md")).ok();
    match (read(source), read(dest)) {
        (Some(a), Some(b)) => crate::skills::hash(&a) != crate::skills::hash(&b),
        // Nothing there yet, or nothing to compare: not a replacement.
        _ => false,
    }
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
/// Where a step writes, whatever kind it is.
pub fn step_target(step: &Step) -> &std::path::Path {
    match step {
        Step::Config(c) => &c.path,
        Step::ConfigRemove(c) => &c.path,
        Step::Skill(s) => &s.dest,
        Step::SkillRemove(s) => &s.path,
    }
}

pub fn preview(plan: &Plan) -> Result<String> {
    use std::fmt::Write as _;
    let mut out = String::new();

    for step in &plan.steps {
        // F-5.10: a write inside a working tree is a write other people will see.
        // Rendered under the path it qualifies, once the path has been printed.
        let git = crate::projects::git_root(step_target(step))
            .map(|root| format!("  in the git repository at {}
", root.display()));

        match step {
            Step::Config(change) => {
                let _ = writeln!(
                    out,
                    "{} <- {} ({})",
                    change.path.display(),
                    change.entry_name,
                    change.providers.join(", ")
                );
                if let Some(note) = &git {
                    out.push_str(note);
                }
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
                if let Some(note) = &git {
                    out.push_str(note);
                }
                let before = std::fs::read_to_string(&removal.path).unwrap_or_default();
                let mut editor = Editor::parse(&before, removal.format)?;
                editor.remove(&removal.pointer, &removal.entry_name)?;
                out.push_str(&unified_diff(&before, &editor.render()));
                out.push('\n');
            }
            Step::SkillRemove(removal) => {
                let _ = writeln!(out, "{}", removal.path.display());
                if let Some(note) = &git {
                    out.push_str(note);
                }
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
                if let Some(note) = &git {
                    out.push_str(note);
                }
                if copy.replaces_divergent {
                    let _ = writeln!(out, "  ! REPLACES an existing copy that differs");
                }
                let _ = writeln!(out, "  + copy from {}", copy.source.display());
                // A library write serves no harness, and "serves: " with
                // nothing after it reads as a bug rather than as a fact.
                if !copy.providers.is_empty() {
                    let _ = writeln!(out, "  serves: {}", copy.providers.join(", "));
                }
                let _ = writeln!(out, "  reason: {}", copy.reason);
                if copy.replaces_divergent
                    && let Ok(lines) = step_diff(step)
                {
                    out.push_str(&crate::diff::render(&lines));
                }
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

/// The diff for one step, as structured lines rather than pre-rendered text.
///
/// The CLI renders these with `+`/`-` signs; the GUI colours them. Both call
/// this, so the two can never show different diffs — which matters, because the
/// preview is the thing the user is trusting.
///
/// Credentials are masked here exactly as they are in [`preview`] (F-7.4).
pub fn step_diff(step: &Step) -> Result<Vec<DiffLine>> {
    let (before, after) = match step {
        Step::Config(change) => render_change(change, Redact::Yes)?,
        Step::ConfigRemove(removal) => {
            let before = std::fs::read_to_string(&removal.path).unwrap_or_default();
            let mut editor = Editor::parse(&before, removal.format)?;
            editor.remove(&removal.pointer, &removal.entry_name)?;
            (before.clone(), editor.render())
        }
        // A skill write that replaces an existing copy destroys content, so show
        // exactly what goes. A write into empty space has nothing to diff.
        Step::Skill(write) if write.replaces_divergent => {
            let read =
                |dir: &Path| std::fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
            (read(&write.dest), read(&write.source))
        }
        Step::Skill(_) | Step::SkillRemove(_) => return Ok(Vec::new()),
    };
    Ok(crate::diff::lines(&before, &after, 3))
}

fn unified_diff(before: &str, after: &str) -> String {
    crate::diff::render(&crate::diff::lines(before, after, 3))
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
    apply_with(plan, &write::Options::default())
}

pub fn apply_with(plan: &Plan, options: &write::Options) -> Vec<Outcome> {
    let mut outcomes = Vec::new();

    for step in &plan.steps {
        match step {
            Step::Config(change) => {
                let result = (|| -> Result<String> {
                    let (_, after) = render_change(change, Redact::No)?;
                    let report = write::save_with(&change.path, &after, options)?;
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
                    let report = write::save_with(&removal.path, &editor.render(), options)?;
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
        // Rendering goes through `crate::diff` now, which writes "  + text".
        let d = unified_diff("a\nb\n", "a\nb\nc\n");
        assert!(d.contains("+ c"), "got:\n{d}");
        assert!(!d.contains("- a"), "got:\n{d}");
        assert!(d.contains("  a"), "context should be kept:\n{d}");
    }

    #[test]
    fn empty_plan_reports_empty() {
        assert!(Plan::default().is_empty());
        assert!(Plan::default().providers_served().is_empty());
    }
}
