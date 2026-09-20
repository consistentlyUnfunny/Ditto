//! View models handed to the frontend.
//!
//! These live here, not in `inception-core`, so the engine stays free of any
//! presentation concern (NF-5.1). Everything below is a projection of a core
//! type — nothing decides anything.

use inception_core::discover::DetectedProvider;
use inception_core::inventory::{DriftStatus, SkillGroup};
use inception_core::diff::{DiffLine, LineKind};
use inception_core::plan::{Plan, Step};
use inception_core::skills::SkillState;
use inception_core::spec::Scope;
use serde::Serialize;

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// Where this harness keeps its config, for the status line.
    pub home: Option<String>,
}

/// A span joining two columns of the matrix.
///
/// This is the mark that carries the product's central insight: a tie means
/// those harnesses are looking at the same content. `shared` means literally the
/// same file on disk; otherwise they are separate copies that happen to agree.
#[derive(Serialize)]
pub struct TieView {
    pub from: usize,
    pub to: usize,
    pub drift: bool,
    pub shared: bool,
}

#[derive(Serialize)]
pub struct CellView {
    /// "on" installed · "off" absent · "bad" present but unreadable
    pub state: &'static str,
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct RowView {
    pub name: String,
    pub kind: &'static str,
    pub status: &'static str,
    pub detail: Option<String>,
    pub cells: Vec<CellView>,
    pub ties: Vec<TieView>,
}

#[derive(Serialize)]
pub struct Overview {
    pub scope: String,
    pub providers: Vec<ProviderView>,
    pub skills: Vec<RowView>,
    pub servers: Vec<RowView>,
}

pub fn providers(found: &[DetectedProvider<'_>]) -> Vec<ProviderView> {
    found
        .iter()
        .map(|p| ProviderView {
            id: p.id().to_string(),
            name: p.spec.name.clone(),
            installed: p.installed,
            home: p.home.as_ref().map(|h| h.display().to_string()),
        })
        .collect()
}

fn status_label(status: DriftStatus) -> &'static str {
    match status {
        DriftStatus::Unique => "unique",
        DriftStatus::Identical => "identical",
        DriftStatus::Drifted => "drifted",
        DriftStatus::Problem => "problem",
    }
}

/// Project one skill group onto the provider columns.
pub fn skill_row(group: &SkillGroup, order: &[String]) -> RowView {
    // provider id -> index of the location serving it
    let mut cells: Vec<CellView> = order
        .iter()
        .map(|_| CellView { state: "off", path: None })
        .collect();
    let mut location_of: Vec<Option<usize>> = vec![None; order.len()];

    for (li, location) in group.locations.iter().enumerate() {
        for provider in &location.providers {
            let Some(index) = order.iter().position(|id| id == provider) else {
                continue;
            };
            cells[index] = CellView {
                state: if location.entry.is_ok() { "on" } else { "bad" },
                path: Some(location.path.display().to_string()),
            };
            location_of[index] = Some(li);
        }
    }

    // Ties join consecutive occupied columns. When both ends resolve to the same
    // location it is one file; otherwise they are separate copies.
    let occupied: Vec<usize> = location_of
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.map(|_| i))
        .collect();
    let drift = group.status == DriftStatus::Drifted;
    let ties = occupied
        .windows(2)
        .map(|pair| TieView {
            from: pair[0],
            to: pair[1],
            drift,
            shared: location_of[pair[0]] == location_of[pair[1]],
        })
        .collect();

    let detail = match group.status {
        DriftStatus::Drifted => Some("these differ".to_string()),
        DriftStatus::Problem => group
            .locations
            .iter()
            .find_map(|l| match &l.entry.state {
                SkillState::MissingSkillMd => Some("no SKILL.md".to_string()),
                SkillState::Invalid(_) => Some("unreadable".to_string()),
                SkillState::Ok(_) => None,
            }),
        _ => None,
    };

    RowView {
        name: group.identity.clone(),
        kind: "skill",
        status: status_label(group.status),
        detail,
        cells,
        ties,
    }
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CopyView {
    pub path: String,
    pub providers: Vec<String>,
    pub description: String,
    pub body: String,
    pub hash: String,
    pub deprecated: bool,
    pub readable: bool,
}

#[derive(Serialize)]
pub struct ComparisonView {
    pub left: String,
    pub right: String,
    pub lines: Vec<DiffLineView>,
}

#[derive(Serialize)]
pub struct DetailView {
    pub name: String,
    pub status: &'static str,
    pub copies: Vec<CopyView>,
    /// Present only when two readable copies exist and actually differ.
    pub comparison: Option<ComparisonView>,
}

pub fn detail(group: &SkillGroup) -> DetailView {
    let comparison = inception_core::inventory::compare_copies(group)
        .filter(|c| inception_core::diff::has_changes(&c.lines))
        .map(|c| ComparisonView {
            left: c.left.display().to_string(),
            right: c.right.display().to_string(),
            lines: c
                .lines
                .iter()
                .map(|l| DiffLineView { kind: l.kind.as_str(), text: l.text.clone() })
                .collect(),
        });

    DetailView {
        name: group.identity.clone(),
        status: status_label(group.status),
        comparison,
        copies: group
            .locations
            .iter()
            .map(|location| {
                let skill = location.entry.skill();
                CopyView {
                    path: location.path.display().to_string(),
                    providers: location.providers.clone(),
                    description: skill.map(|s| s.description.clone()).unwrap_or_default(),
                    // Enough of the body to see, not the whole file.
                    body: skill
                        .map(|s| s.body.trim().chars().take(600).collect())
                        .unwrap_or_default(),
                    hash: skill
                        .map(|s| s.content_hash.chars().take(8).collect())
                        .unwrap_or_else(|| "--------".to_string()),
                    deprecated: location.deprecated,
                    readable: location.entry.is_ok(),
                }
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct DiffLineView {
    pub kind: &'static str,
    pub text: String,
}

#[derive(Serialize)]
pub struct StepView {
    pub kind: &'static str,
    pub target: String,
    pub providers: Vec<String>,
    pub reason: String,
    pub diff: Vec<DiffLineView>,
    pub dropped: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct RefusalView {
    pub provider: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct PlanView {
    pub steps: Vec<StepView>,
    pub refusals: Vec<RefusalView>,
    pub served: Vec<String>,
}

pub fn plan(plan: &Plan) -> PlanView {
    let steps = plan
        .steps
        .iter()
        .map(|step| {
            let diff = inception_core::plan::step_diff(step)
                .unwrap_or_default()
                .into_iter()
                .map(|DiffLine { kind, text }| DiffLineView {
                    kind: match kind {
                        LineKind::Added => "add",
                        LineKind::Removed => "del",
                        LineKind::Context => "ctx",
                    },
                    text,
                })
                .collect();

            match step {
                Step::Config(c) => StepView {
                    kind: "config",
                    target: c.path.display().to_string(),
                    providers: c.providers.clone(),
                    reason: format!("adds {} to this file", c.entry_name),
                    diff,
                    dropped: c.dropped.clone(),
                    warnings: c.warnings.clone(),
                },
                Step::Skill(s) => StepView {
                    kind: "skill",
                    target: s.dest.display().to_string(),
                    providers: s.providers.clone(),
                    reason: s.reason.clone(),
                    diff,
                    dropped: vec![],
                    warnings: if s.replaces_divergent {
                        vec!["replaces an existing copy that differs".to_string()]
                    } else {
                        vec![]
                    },
                },
                Step::ConfigRemove(c) => StepView {
                    kind: "config-remove",
                    target: c.path.display().to_string(),
                    providers: c.providers.clone(),
                    reason: format!("removes {} from this file", c.entry_name),
                    diff,
                    dropped: vec![],
                    warnings: vec![],
                },
                Step::SkillRemove(s) => StepView {
                    kind: "skill-remove",
                    target: s.path.display().to_string(),
                    providers: s.providers.clone(),
                    reason: if s.providers.len() > 1 {
                        format!("shared directory — removes it from {}", s.providers.join(", "))
                    } else {
                        "deletes this directory".to_string()
                    },
                    diff,
                    dropped: vec![],
                    warnings: vec![],
                },
            }
        })
        .collect();

    PlanView {
        steps,
        refusals: plan
            .refusals
            .iter()
            .map(|r| RefusalView {
                provider: r.provider.clone(),
                reason: r.reason.clone(),
            })
            .collect(),
        served: plan.providers_served().iter().map(|s| s.to_string()).collect(),
    }
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct FindingView {
    pub severity: &'static str,
    pub provider: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

pub fn findings(items: Vec<inception_core::doctor::Finding>) -> Vec<FindingView> {
    use inception_core::doctor::Severity;
    items
        .into_iter()
        .map(|f| FindingView {
            severity: match f.severity {
                Severity::Security => "security",
                Severity::Warning => "warning",
                Severity::Info => "info",
            },
            provider: f.provider,
            message: f.message,
            suggestion: f.suggestion,
        })
        .collect()
}

#[derive(Serialize)]
pub struct OutcomeView {
    pub target: String,
    pub providers: Vec<String>,
    pub ok: bool,
    pub message: String,
}

pub fn scope_label(scope: Scope) -> String {
    match scope {
        Scope::User => "user scope".to_string(),
        Scope::Project => "project scope".to_string(),
        Scope::Local => "local scope".to_string(),
    }
}
