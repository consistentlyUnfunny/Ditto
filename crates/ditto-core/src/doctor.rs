//! Configuration health checks (T-3.16, F-8, NF-4.4).
//!
//! Read-only. `doctor` reports; it never fixes anything on its own, because the
//! things it finds — a deprecated path, a plaintext key — are the user's
//! decisions to make.

use crate::discover::DetectedProvider;
use crate::inventory::{self, DriftStatus};
use crate::mcp;
use crate::skills::SkillState;
use crate::spec::Scope;
use crate::tree::FileState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing, nothing wrong.
    Info,
    /// Something will probably bite later.
    Warning,
    /// A credential or a permission grant.
    Security,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warn",
            Severity::Security => "SECURITY",
        }
    }
}

/// Something a frontend can offer to do about a finding.
///
/// Every suggestion already names an action in prose; carrying it as data lets
/// the GUI put a button on it instead of leaving the user to go and find the
/// thing themselves.
#[derive(Debug, Clone)]
pub enum Action {
    /// Open this artifact's detail view, by name.
    Open(String),
    /// Show this path in the OS file manager.
    Reveal(std::path::PathBuf),
}

#[derive(Debug)]
pub struct Finding {
    pub severity: Severity,
    pub provider: Option<String>,
    pub message: String,
    /// What to do about it, when there is an obvious answer.
    pub suggestion: Option<String>,
    pub action: Option<Action>,
}

/// Who holds a differing copy, for the drift message.
fn disagreeing(group: &inventory::SkillGroup) -> Vec<String> {
    let mut who: Vec<String> = group.providers().iter().map(|p| (*p).to_string()).collect();
    if group.locations.iter().any(|l| l.library) {
        who.insert(0, "your library".to_string());
    }
    who
}

/// Run every check. Findings come back most severe first.
///
/// `library` is `~/.ditto/library` when enabled, as for
/// [`inventory::skills`] — a library copy that disagrees with the harnesses is
/// drift worth reporting, so it has to be in scope here too.
pub fn check(
    providers: &[DetectedProvider<'_>],
    scope: Scope,
    library: Option<&std::path::Path>,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    for provider in providers {
        if !provider.installed {
            continue;
        }

        // A config we cannot parse is also a config we will refuse to write.
        for file in provider.invalid_files() {
            if let FileState::Invalid(reason) = &file.state {
                findings.push(Finding {
                    severity: Severity::Warning,
                    provider: Some(provider.id().to_string()),
                    message: format!("{} cannot be parsed: {reason}", file.path.display()),
                    suggestion: Some(
                        "deploys to this harness will be refused until it parses".to_string(),
                    ),
                    action: Some(Action::Reveal(file.path.clone())),
                });
            }
        }

        // Two standing-instruction files loaded by one harness (T-6.8).
        //
        // Invisible from inside any single harness: each just loads what it
        // finds. The contradiction only shows up in the agent's behaviour.
        let loaded: Vec<&crate::discover::ResolvedInstructions> =
            provider.existing_instructions(scope).collect();
        if loaded.len() > 1 {
            let names = loaded
                .iter()
                .map(|f| file_name(&f.path))
                .collect::<Vec<_>>()
                .join(" and ");
            // Declared order is read order, so the first one is the one that wins
            // where they disagree.
            let first = file_name(&loaded[0].path);
            findings.push(Finding {
                severity: Severity::Warning,
                provider: Some(provider.id().to_string()),
                message: format!("{} loads both {names} as standing instructions", provider.spec.name),
                suggestion: Some(format!(
                    "{first} is read first; keep one, or make sure they agree"
                )),
                action: Some(Action::Reveal(loaded[0].path.clone())),
            });
        }

        // Skills sitting in a location the harness no longer documents.
        for dir in provider.deprecated_in_use() {
            let canonical = provider
                .skill_dirs
                .iter()
                .find(|d| d.scope == dir.scope && d.canonical)
                .map(|d| d.path.display().to_string());
            findings.push(Finding {
                severity: Severity::Warning,
                provider: Some(provider.id().to_string()),
                message: format!("{} is a deprecated skill location", dir.path.display()),
                suggestion: canonical.map(|c| {
                    format!("move these skills to {c}, where more harnesses can see them")
                }),
                action: Some(Action::Reveal(dir.path.clone())),
            });
        }
    }

    // --- MCP servers ------------------------------------------------------
    for location in inventory::mcp_servers(providers, scope) {
        let server = &location.server;

        // NF-4.4: credential-shaped values stored in cleartext.
        let secrets = server.secret_like();
        if !secrets.is_empty() {
            findings.push(Finding {
                severity: Severity::Security,
                provider: Some(location.provider.clone()),
                message: format!(
                    "server '{}' stores {} in plaintext in {}",
                    server.name,
                    secrets.join(", "),
                    location.file.display()
                ),
                // `dit import` adopts *skills* into the library. It does
                // not yet promote a server's literal to a keychain reference,
                // because a server has nowhere canonical to live until Q-12 is
                // answered. Pointing at a command that will not do this would be
                // worse than naming the exposure and stopping there.
                suggestion: Some(
                    "rotate this key if it has been shared — copying this server to another harness puts another plaintext copy on disk"
                        .to_string(),
                ),
                action: Some(Action::Open(server.name.clone())),
            });
        }

        // Gemini's `trust = true` disables per-tool confirmation.
        if server.extra.get("trust").and_then(crate::Value::as_bool) == Some(true) {
            findings.push(Finding {
                severity: Severity::Security,
                provider: Some(location.provider.clone()),
                message: format!(
                    "server '{}' sets trust = true, which bypasses tool-call confirmation",
                    server.name
                ),
                suggestion: None,
                action: Some(Action::Open(server.name.clone())),
            });
        }

        // Typo protection: a key close to a documented one but not quite it.
        if let Some(spec) = providers
            .iter()
            .find(|p| p.id() == location.provider)
            .and_then(|p| p.spec.mcp.as_ref())
            && !spec.extra.known.is_empty()
        {
            for key in server.extra.keys() {
                if spec.extra.known.iter().any(|k| k == key) {
                    continue;
                }
                if let Some(near) = spec.extra.known.iter().find(|k| close(k, key)) {
                    findings.push(Finding {
                        severity: Severity::Warning,
                        provider: Some(location.provider.clone()),
                        message: format!(
                            "server '{}' has key '{key}', which looks like a typo",
                            server.name
                        ),
                        suggestion: Some(format!("did you mean '{near}'?")),
                        action: Some(Action::Reveal(location.file.clone())),
                    });
                }
            }
        }
    }

    // --- Skills -----------------------------------------------------------
    for group in inventory::skills(providers, scope, library) {
        match group.status {
            // Name the library explicitly when it is one of the sides. It reads
            // as no harness at all otherwise, so "differs between codex" would
            // be the whole message for the commonest case the library creates.
            DriftStatus::Drifted => findings.push(Finding {
                severity: Severity::Warning,
                provider: None,
                message: format!(
                    "skill '{}' differs between {}",
                    group.identity,
                    disagreeing(&group).join(" and ")
                ),
                suggestion: Some(
                    "open it to compare the versions and pick which one wins".to_string(),
                ),
                action: Some(Action::Open(group.identity.clone())),
            }),
            DriftStatus::Problem => {
                for location in &group.locations {
                    let detail = match &location.entry.state {
                        SkillState::MissingSkillMd => "has no SKILL.md",
                        SkillState::Invalid(_) => "could not be read",
                        SkillState::Ok(_) => continue,
                    };
                    findings.push(Finding {
                        severity: Severity::Warning,
                        provider: None,
                        message: format!("{} {detail}", location.path.display()),
                        suggestion: Some(
                            "the harness will ignore this directory; remove it or add a SKILL.md"
                                .to_string(),
                        ),
                        action: Some(Action::Reveal(location.path.clone())),
                    });
                }
            }
            DriftStatus::Identical | DriftStatus::Unique => {}
        }
    }

    // Most severe first. `Reverse` because the derived ordering puts Info first.
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    findings
}

/// The last component of a path, for a message that names files rather than
/// printing two absolute paths the user already knows the directory of.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Cheap near-match: same length, differing in at most two characters, or one is
/// the other with a character dropped. Enough to catch `startup_timout_sec`
/// without pulling in an edit-distance crate.
fn close(known: &str, candidate: &str) -> bool {
    if known == candidate {
        return false;
    }
    let (a, b) = (known.as_bytes(), candidate.as_bytes());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    if a.len() == b.len() {
        let differing = a.iter().zip(b).filter(|(x, y)| x != y).count();
        return differing > 0 && differing <= 2;
    }
    // One character inserted or deleted.
    let (long, short) = if a.len() > b.len() { (a, b) } else { (b, a) };
    let mut skipped = false;
    let mut j = 0;
    for &c in long {
        if j < short.len() && short[j] == c {
            j += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
        }
    }
    j == short.len()
}

/// Whether a stored reference still resolves, for a keychain health check.
pub fn dangling_references(
    providers: &[DetectedProvider<'_>],
    scope: Scope,
    store: &dyn crate::secrets::SecretStore,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for location in inventory::mcp_servers(providers, scope) {
        for (key, secret) in location.server.env.iter().chain(location.server.headers.iter()) {
            if let mcp::Secret::Ref(name) = secret
                && !store.exists(name)
            {
                findings.push(Finding {
                    severity: Severity::Warning,
                    provider: Some(location.provider.clone()),
                    message: format!(
                        "server '{}' references secret '{name}' for {key}, which is not in the \
                         keychain",
                        location.server.name
                    ),
                    suggestion: Some("deploys involving this server will fail".to_string()),
                    action: Some(Action::Open(location.server.name.clone())),
                });
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_match_catches_a_dropped_character() {
        assert!(close("startup_timeout_sec", "startup_timout_sec"));
    }

    #[test]
    fn near_match_catches_a_transposition() {
        assert!(close("tool_timeout_sec", "tool_timeuot_sec"));
    }

    #[test]
    fn near_match_ignores_identical_and_unrelated() {
        assert!(!close("startup_timeout_sec", "startup_timeout_sec"));
        assert!(!close("startup_timeout_sec", "command"));
        assert!(!close("trust", "includeTools"));
    }

    #[test]
    fn severity_orders_security_highest() {
        let mut all = [Severity::Info, Severity::Security, Severity::Warning];
        all.sort_by(|a, b| b.cmp(a));
        assert_eq!(all[0], Severity::Security);
        assert_eq!(all[2], Severity::Info);
    }
}
