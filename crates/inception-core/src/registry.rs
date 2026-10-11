//! Loading and validating provider specs.
//!
//! Built-in specs are compiled into the binary with `include_str!`, so the tool
//! is a single file with no data directory to install. User specs in
//! `~/.inception/providers/` override built-ins by `id` (F-1.2).

use crate::error::{Error, Result};
use crate::pointer;
use crate::spec::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `include_str!` inlines the file contents at compile time, so these are checked
/// to exist when the crate builds — a missing spec is a compile error, not a
/// runtime surprise.
const BUILTIN: &[(&str, &str)] = &[
    (
        "claude-code",
        include_str!("../../../providers/claude-code.provider.toml"),
    ),
    ("codex", include_str!("../../../providers/codex.provider.toml")),
    (
        "opencode",
        include_str!("../../../providers/opencode.provider.toml"),
    ),
    (
        "gemini-cli",
        include_str!("../../../providers/gemini-cli.provider.toml"),
    ),
    (
        "copilot-cli",
        include_str!("../../../providers/copilot-cli.provider.toml"),
    ),
];

/// A spec that failed to load. Kept rather than discarded so the CLI can report
/// it: per F-1.4 one bad spec disables one provider, it never aborts startup.
#[derive(Debug)]
pub struct RejectedSpec {
    pub source: String,
    pub error: Error,
}

#[derive(Debug, Default)]
pub struct Registry {
    specs: BTreeMap<String, ProviderSpec>,
    pub rejected: Vec<RejectedSpec>,
}

impl Registry {
    /// Built-in specs only. Used by tests and by `validate`.
    pub fn builtin() -> Registry {
        let mut registry = Registry::default();
        for (id, text) in BUILTIN {
            match parse_and_validate(text, Path::new(id)) {
                Ok(spec) => {
                    registry.specs.insert(spec.id.clone(), spec);
                }
                // A broken built-in is a bug in this repo, not user error. It is
                // still surfaced rather than panicking, and CI fails on it (F-1.5).
                Err(error) => registry.rejected.push(RejectedSpec {
                    source: format!("builtin:{id}"),
                    error,
                }),
            }
        }
        registry
    }

    /// Built-ins plus any user specs, with user specs winning on `id` collision.
    pub fn load() -> Registry {
        let mut registry = Registry::builtin();
        let Some(dir) = user_provider_dir() else {
            return registry;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return registry; // absent directory is normal, not an error
        };

        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(".provider.toml"))
            })
            .collect();
        paths.sort(); // deterministic load order

        for path in paths {
            let result = std::fs::read_to_string(&path)
                .map_err(Error::reading(&path))
                .and_then(|text| parse_and_validate(&text, &path));
            match result {
                Ok(spec) => {
                    registry.specs.insert(spec.id.clone(), spec);
                }
                Err(error) => registry.rejected.push(RejectedSpec {
                    source: path.display().to_string(),
                    error,
                }),
            }
        }
        registry
    }

    pub fn get(&self, id: &str) -> Option<&ProviderSpec> {
        self.specs.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProviderSpec> {
        self.specs.values()
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

pub fn user_provider_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".inception").join("providers"))
}

pub fn parse_and_validate(text: &str, path: &Path) -> Result<ProviderSpec> {
    let spec: ProviderSpec = toml::from_str(text).map_err(|source| Error::SpecParse {
        path: path.to_path_buf(),
        source,
    })?;
    validate(&spec)?;
    Ok(spec)
}

// ---------------------------------------------------------------------------
// Validation — PROVIDER-SPEC.md §9
// ---------------------------------------------------------------------------

/// Every rule here exists because getting it wrong produces a *silent* misbehaviour
/// rather than a crash. Unknown-key typos are already caught by
/// `deny_unknown_fields` on the structs themselves.
pub fn validate(spec: &ProviderSpec) -> Result<()> {
    let id = &spec.id;

    if spec.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(Error::SpecVersion {
            provider: id.clone(),
            found: spec.schema_version,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }

    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::invalid(
            id,
            "id must be a non-empty lowercase slug (a-z, 0-9, '-')",
        ));
    }

    // Duplicate file ids would make `file = "..."` references ambiguous.
    let mut seen = std::collections::BTreeSet::new();
    for file in &spec.files {
        if !seen.insert(&file.id) {
            return Err(Error::invalid(id, format!("duplicate file id '{}'", file.id)));
        }
        if file.path.as_slice().is_empty() {
            return Err(Error::invalid(
                id,
                format!("file '{}' declares no path candidates", file.id),
            ));
        }
    }

    if let Some(mcp) = &spec.mcp {
        validate_mcp(spec, mcp)?;
    }
    if let Some(skills) = &spec.skills {
        validate_skills(spec, skills)?;
    }
    if let Some(projects) = &spec.projects {
        if spec.file(&projects.file).is_none() {
            return Err(Error::invalid(
                id,
                format!("projects references unknown file id '{}'", projects.file),
            ));
        }
        pointer::validate(&projects.pointer)
            .map_err(|e| Error::invalid(id, format!("projects pointer is invalid: {e}")))?;
    }

    // Two declarations of the same instruction path would double-count in the
    // shadowing check and make its message nonsense.
    let mut seen = std::collections::BTreeSet::new();
    for file in &spec.instructions {
        if !seen.insert((file.scope.as_str(), file.path.as_str())) {
            return Err(Error::invalid(
                id,
                format!("duplicate instructions path '{}'", file.path),
            ));
        }
    }
    Ok(())
}

fn validate_mcp(spec: &ProviderSpec, mcp: &McpSpec) -> Result<()> {
    let id = &spec.id;

    if mcp.supported && mcp.locations.is_empty() {
        return Err(Error::invalid(id, "mcp.supported is true but no locations declared"));
    }

    for location in &mcp.locations {
        if spec.file(&location.file).is_none() {
            return Err(Error::invalid(
                id,
                format!(
                    "mcp location references unknown file id '{}'",
                    location.file
                ),
            ));
        }
        pointer::validate(&location.pointer).map_err(|e| {
            Error::invalid(id, format!("mcp location pointer is invalid: {e}"))
        })?;
    }

    // `argv-array` folds another field into this one, so it needs to say which.
    for (canonical, field) in &mcp.fields {
        if field.form == Some(Form::ArgvArray) && field.absorbs.is_none() {
            return Err(Error::invalid(
                id,
                format!("field '{canonical}' uses form 'argv-array' but declares no 'absorbs'"),
            ));
        }
        if let Some(absorbed) = &field.absorbs {
            if mcp.fields.contains_key(absorbed) {
                return Err(Error::invalid(
                    id,
                    format!(
                        "field '{canonical}' absorbs '{absorbed}', but '{absorbed}' is also \
                         mapped in [mcp.fields] — it must be one or the other"
                    ),
                ));
            }
            if field.form != Some(Form::ArgvArray) {
                return Err(Error::invalid(
                    id,
                    format!("field '{canonical}' declares 'absorbs' without form 'argv-array'"),
                ));
            }
        }
    }

    // Emitting a transport the harness cannot speak would write a broken entry.
    let supported = &mcp.capabilities.transports;
    let declared: Vec<Transport> = match &mcp.transport {
        TransportSpec::TypeValue { write, absent, .. } => write
            .keys()
            .copied()
            .chain(std::iter::once(*absent))
            .collect(),
        TransportSpec::KeyPresence { keys, absent } => {
            keys.keys().copied().chain(std::iter::once(*absent)).collect()
        }
        TransportSpec::UrlPresence { remote, .. } => vec![Transport::Stdio, *remote],
    };
    for transport in declared {
        if !supported.contains(&transport) {
            return Err(Error::invalid(
                &spec.id,
                format!(
                    "transport '{}' appears in the transport strategy but is missing from \
                     capabilities.transports",
                    transport.as_str()
                ),
            ));
        }
    }

    Ok(())
}

fn validate_skills(spec: &ProviderSpec, skills: &SkillsSpec) -> Result<()> {
    let id = &spec.id;

    if skills.supported && skills.dirs.is_empty() {
        return Err(Error::invalid(id, "skills.supported is true but no dirs declared"));
    }

    for scope in [Scope::User, Scope::Project, Scope::Local] {
        let canonical: Vec<&SkillDir> = skills
            .dirs
            .iter()
            .filter(|d| d.scope == scope && d.canonical)
            .collect();
        if canonical.len() > 1 {
            return Err(Error::invalid(
                id,
                format!(
                    "{:?} scope has {} skill dirs marked canonical; exactly one may be",
                    scope,
                    canonical.len()
                ),
            ));
        }
        for dir in &canonical {
            if dir.deprecated {
                return Err(Error::invalid(
                    id,
                    format!("skill dir '{}' is both canonical and deprecated", dir.path),
                ));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// F-1.5: every shipped spec must load and validate.
    #[test]
    fn all_builtin_specs_are_valid() {
        let registry = Registry::builtin();
        assert!(
            registry.rejected.is_empty(),
            "built-in specs failed to load: {:#?}",
            registry.rejected
        );
        assert_eq!(registry.len(), 5, "expected five built-in providers");
    }

    #[test]
    fn builtin_specs_cover_the_expected_providers() {
        let registry = Registry::builtin();
        for id in ["claude-code", "codex", "opencode", "gemini-cli", "copilot-cli"] {
            assert!(registry.get(id).is_some(), "missing provider '{id}'");
        }
    }

    /// The finding that drives F-5.2: only Claude Code refuses ~/.agents/skills.
    #[test]
    fn only_claude_code_lacks_the_shared_skills_dir() {
        let registry = Registry::builtin();
        for spec in registry.iter() {
            let has_shared = spec
                .skill_dirs(Scope::User)
                .iter()
                .any(|d| d.shared && d.path.contains(".agents/skills"));
            if spec.id == "claude-code" {
                assert!(!has_shared, "Claude Code must not declare ~/.agents/skills");
            } else {
                assert!(has_shared, "{} should read ~/.agents/skills", spec.id);
            }
        }
    }

    #[test]
    fn every_provider_declares_exactly_one_canonical_user_skill_dir() {
        let registry = Registry::builtin();
        for spec in registry.iter() {
            let n = spec
                .skill_dirs(Scope::User)
                .iter()
                .filter(|d| d.canonical)
                .count();
            assert_eq!(n, 1, "{} has {n} canonical user skill dirs", spec.id);
        }
    }

    fn parse(text: &str) -> Result<ProviderSpec> {
        parse_and_validate(text, Path::new("test"))
    }

    const MINIMAL: &str = r#"
schema_version = 1
id = "test"
name = "Test"
"#;

    #[test]
    fn minimal_spec_is_valid() {
        assert!(parse(MINIMAL).is_ok());
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let text = format!("{MINIMAL}\nnonsense = true\n");
        assert!(parse(&text).is_err());
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let text = MINIMAL.replace("schema_version = 1", "schema_version = 99");
        assert!(matches!(parse(&text), Err(Error::SpecVersion { .. })));
    }

    #[test]
    fn rejects_location_with_unknown_file_id() {
        let text = format!(
            r#"{MINIMAL}
[[files]]
id = "global"
scope = "user"
format = "toml"
path = "~/x.toml"

[mcp]
[[mcp.locations]]
file = "nope"
scope = "user"
pointer = "/servers"

[mcp.transport]
strategy = "url-presence"
key = "url"
remote = "http"
"#
        );
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("unknown file id"), "got: {err}");
    }

    #[test]
    fn rejects_argv_array_without_absorbs() {
        let text = format!(
            r#"{MINIMAL}
[[files]]
id = "global"
scope = "user"
format = "jsonc"
path = "~/x.json"

[mcp]
[[mcp.locations]]
file = "global"
scope = "user"
pointer = "/mcp"

[mcp.transport]
strategy = "url-presence"
key = "url"
remote = "http"

[mcp.fields]
command = {{ key = "command", form = "argv-array" }}
"#
        );
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("absorbs"), "got: {err}");
    }

    #[test]
    fn rejects_transport_missing_from_capabilities() {
        let text = format!(
            r#"{MINIMAL}
[[files]]
id = "global"
scope = "user"
format = "jsonc"
path = "~/x.json"

[mcp]
[[mcp.locations]]
file = "global"
scope = "user"
pointer = "/mcp"

[mcp.transport]
strategy = "url-presence"
key = "url"
remote = "sse"

[mcp.capabilities]
transports = ["stdio"]
"#
        );
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("capabilities.transports"), "got: {err}");
    }

    #[test]
    fn rejects_two_canonical_dirs_in_one_scope() {
        let text = format!(
            r#"{MINIMAL}
[skills]
layout = "dir-with-skill-md"

[[skills.dirs]]
scope = "user"
path = "~/a"
canonical = true

[[skills.dirs]]
scope = "user"
path = "~/b"
canonical = true
"#
        );
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("canonical"), "got: {err}");
    }
}
