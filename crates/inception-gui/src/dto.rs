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
    /// A copy sits somewhere the harness no longer documents. The CLI has
    /// printed this for a while; the window used to hide it entirely.
    pub deprecated: bool,
    /// Path of that deprecated copy, for the tooltip.
    pub deprecated_path: Option<String>,
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

    let version_count = inception_core::inventory::versions(group).len();
    let deprecated_at = group.locations.iter().find(|l| l.deprecated);

    let detail = match group.status {
        // Say how many versions once there are more than two, since "these
        // differ" stops being precise.
        DriftStatus::Drifted if version_count > 2 => Some(format!("{version_count} versions")),
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
        deprecated: deprecated_at.is_some(),
        deprecated_path: deprecated_at.map(|l| l.path.display().to_string()),
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
pub struct SideView {
    pub path: String,
    pub providers: Vec<String>,
    pub hash: String,
}

#[derive(Serialize)]
pub struct ComparisonView {
    pub left: SideView,
    pub right: SideView,
    pub lines: Vec<DiffLineView>,
}

/// One MCP server, as the drawer shows it.
///
/// Values of credential-shaped variables are never included — only their names
/// (F-7.4). The drawer needs to show *that* a server carries a secret, never
/// what it is.
#[derive(Serialize)]
pub struct ServerView {
    pub name: String,
    pub provider: String,
    pub transport: String,
    pub command: String,
    pub url: Option<String>,
    pub env_names: Vec<String>,
    pub secret_names: Vec<String>,
    /// Harness-specific keys preserved on a round-trip.
    pub extra_keys: Vec<String>,
    pub file: String,
    pub scope: String,
    pub project: Option<String>,
}

/// What the drawer is showing. Tagged so the frontend can switch on `kind`
/// rather than guess from which fields happen to be present.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DetailView {
    // Both boxed so neither variant makes every DetailView as large as the
    // bigger of the two.
    Skill(Box<SkillDetail>),
    Server(Box<ServerView>),
}

/// One distinct version of a skill, with every place it currently lives.
///
/// The resolve flow is a choice between *versions*, not between locations:
/// four copies holding two versions is a two-way decision.
#[derive(Serialize)]
pub struct VersionView {
    pub hash: String,
    pub description: String,
    /// Every harness that currently sees this version.
    pub providers: Vec<String>,
    pub paths: Vec<String>,
    /// The path a deploy uses when this version is chosen.
    pub source: String,
    pub is_default: bool,
}

/// A harness the artifact could be sent to, and whether it already has it.
#[derive(Serialize)]
pub struct TargetView {
    pub id: String,
    pub name: String,
    pub has_it: bool,
}

#[derive(Serialize)]
pub struct SkillDetail {
    pub name: String,
    pub status: &'static str,
    pub copies: Vec<CopyView>,
    /// Present only when two readable copies exist and actually differ.
    pub comparison: Option<ComparisonView>,
    /// The copy a deploy would use as its source. Shown because the choice is
    /// currently implicit and arbitrary — the user deserves to see it even
    /// before they can change it (T-5.14).
    pub source: Option<String>,
    /// How many distinct versions exist across the copies.
    pub version_count: usize,
    /// True when no copy can be read, so there is nothing to deploy.
    pub unusable: bool,
    /// The distinct versions, most widely installed first.
    pub versions: Vec<VersionView>,
    /// Every installed harness, and whether it already has this.
    pub targets: Vec<TargetView>,
}

pub fn server_detail(location: &inception_core::inventory::McpLocation) -> ServerView {
    let s = &location.server;
    ServerView {
        name: s.name.clone(),
        provider: location.provider.clone(),
        transport: s.transport.as_str().to_string(),
        command: s.command_line(),
        url: s.url.clone(),
        env_names: s.env.iter().map(|(k, _)| k.clone()).collect(),
        secret_names: s.secret_like().into_iter().map(str::to_string).collect(),
        extra_keys: s.extra.keys().map(str::to_string).collect(),
        file: location.file.display().to_string(),
        scope: format!("{:?}", location.scope).to_lowercase(),
        project: location.project.clone(),
    }
}

pub fn detail(group: &SkillGroup, providers: &[DetectedProvider<'_>]) -> DetailView {
    DetailView::Skill(Box::new(skill_detail(group, providers)))
}

fn skill_detail(group: &SkillGroup, providers: &[DetectedProvider<'_>]) -> SkillDetail {
    let comparison = inception_core::inventory::compare_copies(group)
        .filter(|c| inception_core::diff::has_changes(&c.lines))
        .map(|c| ComparisonView {
            left: SideView {
                path: c.left.path.display().to_string(),
                providers: c.left.providers.clone(),
                hash: c.left.hash.clone(),
            },
            right: SideView {
                path: c.right.path.display().to_string(),
                providers: c.right.providers.clone(),
                hash: c.right.hash.clone(),
            },
            lines: c
                .lines
                .iter()
                .map(|l| DiffLineView { kind: l.kind.as_str(), text: l.text.clone() })
                .collect(),
        });

    let default_path = inception_core::inventory::default_source(group)
        .map(|l| l.path.display().to_string());

    let mut versions: Vec<VersionView> = inception_core::inventory::versions(group)
        .into_iter()
        .map(|locations| {
            let first = locations[0];
            let mut who: Vec<String> =
                locations.iter().flat_map(|l| l.providers.clone()).collect();
            who.sort();
            who.dedup();
            let source = first.path.display().to_string();
            VersionView {
                hash: first
                    .entry
                    .skill()
                    .map(|s| s.content_hash.chars().take(8).collect())
                    .unwrap_or_default(),
                description: first
                    .entry
                    .skill()
                    .map(|s| s.description.clone())
                    .unwrap_or_default(),
                providers: who,
                paths: locations.iter().map(|l| l.path.display().to_string()).collect(),
                is_default: Some(&source) == default_path.as_ref(),
                source,
            }
        })
        .collect();
    // Widest reach first: the version most harnesses already see is the one a
    // user is most likely to keep.
    versions.sort_by_key(|v| std::cmp::Reverse(v.providers.len()));

    let present: Vec<&str> = group.providers();
    let targets = providers
        .iter()
        .filter(|p| p.installed)
        .map(|p| TargetView {
            id: p.id().to_string(),
            name: p.spec.name.clone(),
            has_it: present.contains(&p.id()),
        })
        .collect();

    SkillDetail {
        versions,
        targets,
        name: group.identity.clone(),
        status: status_label(group.status),
        comparison,
        source: inception_core::inventory::default_source(group)
            .map(|l| l.path.display().to_string()),
        version_count: inception_core::inventory::versions(group).len(),
        unusable: !group.locations.iter().any(|l| l.entry.is_ok()),
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
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ActionView {
    Open { name: String },
    Reveal { path: String },
}

#[derive(Serialize)]
pub struct FindingView {
    pub severity: &'static str,
    pub provider: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
    pub action: Option<ActionView>,
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
            action: f.action.map(|a| match a {
                inception_core::doctor::Action::Open(name) => ActionView::Open { name },
                inception_core::doctor::Action::Reveal(path) => {
                    ActionView::Reveal { path: path.display().to_string() }
                }
            }),
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
