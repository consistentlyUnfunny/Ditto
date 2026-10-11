//! Provider specs — the data model for `providers/*.provider.toml`.
//!
//! This module is a direct transcription of `docs/PROVIDER-SPEC.md`. If the two
//! ever disagree, the document is the specification and this file is the bug.
//!
//! Rust notes for readers coming from Java/TS:
//!   - `#[derive(Deserialize)]` is Jackson/`JSON.parse` with a schema: it generates
//!     the parser from the struct shape.
//!   - `#[serde(deny_unknown_fields)]` makes a typo in a spec a hard error instead
//!     of a silently ignored key. That is requirement F-1.3.
//!   - `Option<T>` is a value that may be absent — there is no `null`.
//!   - `enum` here is a *tagged union* (like a TS discriminated union), not a
//!     Java enum: each variant can carry different data.

use serde::Deserialize;
use std::collections::BTreeMap;

/// The only schema version this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSpec {
    pub schema_version: u32,
    /// Stable slug used in CLI arguments, e.g. `codex`.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub home: Option<HomeSpec>,
    /// `[[files]]` blocks.
    #[serde(default)]
    pub files: Vec<FileSpec>,
    #[serde(default)]
    pub mcp: Option<McpSpec>,
    #[serde(default)]
    pub skills: Option<SkillsSpec>,
    /// Where this harness records the projects the user works in.
    #[serde(default)]
    pub projects: Option<ProjectsSpec>,
    /// `[[instructions]]` blocks — the prose files a harness loads as standing
    /// instructions, in the order it reads them.
    #[serde(default)]
    pub instructions: Vec<InstructionFile>,
}

/// A standing-instructions file, e.g. the cross-harness `AGENTS.md` convention
/// or a harness's own equivalent.
///
/// We never read, write or merge these — the engine's scope is skills and MCP.
/// They are declared only so `doctor` can see when one harness is loading two
/// of them at once, which is a real source of contradictory instructions and is
/// invisible from inside any single harness.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionFile {
    pub scope: Scope,
    pub path: String,
    /// Part of a convention other harnesses also read.
    #[serde(default)]
    pub shared: bool,
}

/// Where a harness keeps its list of known projects.
///
/// Claude Code and Codex both maintain a map keyed by absolute project path —
/// one for per-project settings, one for trust levels. Reading them lets the
/// user pick a project instead of typing a path, and declaring it here keeps
/// the engine free of harness names.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectsSpec {
    /// References a `[[files]]` id.
    pub file: String,
    /// RFC 6901 pointer to a map whose *keys* are project paths.
    pub pointer: String,
}

/// How to find the harness's base directory.
///
/// `env` wins over `default` when the variable is set (F-2.3) — this is how
/// `CODEX_HOME` and `COPILOT_HOME` are honoured.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeSpec {
    #[serde(default)]
    pub env: Option<String>,
    pub default: String,
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSpec {
    pub id: String,
    pub scope: Scope,
    pub format: Format,
    /// One path, or several candidates tried in order (PROVIDER-SPEC §3).
    pub path: PathCandidates,
}

/// A single path or a list of them.
///
/// `#[serde(untagged)]` means "try each variant until one fits", so both
/// `path = "a"` and `path = ["a", "b"]` deserialize into this.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PathCandidates {
    One(String),
    Many(Vec<String>),
}

impl PathCandidates {
    /// Always view it as a list, so callers never branch on the shape.
    pub fn as_slice(&self) -> &[String] {
        match self {
            PathCandidates::One(p) => std::slice::from_ref(p),
            PathCandidates::Many(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
    /// Claude Code only: stored in the user file but namespaced per project.
    Local,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Toml,
    /// `json` is a deliberate alias for `jsonc`: real `.json` config files contain
    /// `//` comments in the wild (RESEARCH §4), so we never use a strict parser.
    #[serde(alias = "json")]
    Jsonc,
    Yaml,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Toml => "TOML",
            Format::Jsonc => "JSONC",
            Format::Yaml => "YAML",
        }
    }
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSpec {
    #[serde(default = "yes")]
    pub supported: bool,
    /// `[[mcp.locations]]` blocks.
    #[serde(default)]
    pub locations: Vec<McpLocation>,
    pub transport: TransportSpec,
    /// Canonical field name -> where it lives in this harness's files.
    #[serde(default)]
    pub fields: BTreeMap<String, FieldSpec>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub extra: ExtraSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpLocation {
    /// References a `[[files]]` id.
    pub file: String,
    pub scope: Scope,
    /// RFC 6901 JSON Pointer to the map of name -> server.
    pub pointer: String,
}

/// How this harness says which transport a server uses.
///
/// Three strategies because harnesses genuinely do this three different ways
/// (RESEARCH §1). The `strategy` key selects the variant.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "strategy", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TransportSpec {
    /// A field's *value* names the transport. Claude Code, OpenCode, Copilot.
    TypeValue {
        key: String,
        /// What a missing key means.
        absent: Transport,
        /// Harness value -> canonical.
        #[serde(default)]
        read: BTreeMap<String, Transport>,
        /// Canonical -> harness value. Separate from `read` because the relation
        /// is not 1:1 — Claude accepts `http` and `streamable-http`, emits `http`.
        #[serde(default)]
        write: BTreeMap<Transport, WriteRule>,
    },
    /// *Which key exists* names the transport. Gemini CLI: `url` = SSE,
    /// `httpUrl` = streamable HTTP. No value mapping can express this.
    KeyPresence {
        absent: Transport,
        /// Canonical transport -> the key whose presence implies it.
        keys: BTreeMap<Transport, String>,
    },
    /// Remote if a url field is set, else stdio. Codex.
    UrlPresence { key: String, remote: Transport },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
    /// Claude Code only.
    Ws,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
            Transport::Ws => "ws",
        }
    }

    pub fn is_remote(self) -> bool {
        !matches!(self, Transport::Stdio)
    }
}

/// Either a literal value to write, or `{ omit = true }` to write no key at all
/// (how stdio is expressed in harnesses that only tag remote entries).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum WriteRule {
    Value(String),
    Omit { omit: bool },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    /// The key this canonical field uses in the harness's file.
    pub key: String,
    #[serde(default)]
    pub form: Option<Form>,
    /// For `form = "argv-array"`: the canonical field folded into this one.
    #[serde(default)]
    pub absorbs: Option<String>,
    #[serde(default)]
    pub default: Option<toml::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Form {
    String,
    /// `["uvx", "mcp-obsidian"]` — element 0 is the command, the rest are args.
    ArgvArray,
    List,
    Map,
    Bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub transports: Vec<Transport>,
    #[serde(default = "yes")]
    pub headers: bool,
    #[serde(default = "yes")]
    pub env: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities {
            transports: vec![Transport::Stdio],
            headers: true,
            env: true,
        }
    }
}

/// Documentation and typo-checking only. Unlisted keys are still preserved —
/// preservation is unconditional (PROVIDER-SPEC §6).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraSpec {
    #[serde(default)]
    pub known: Vec<String>,
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsSpec {
    #[serde(default = "yes")]
    pub supported: bool,
    pub layout: SkillLayout,
    /// Glob patterns to skip. Default `.*` hides harness-managed dirs such as
    /// `~/.codex/skills/.system/` (F-3.2).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// `[[skills.dirs]]` blocks.
    #[serde(default)]
    pub dirs: Vec<SkillDir>,
    #[serde(default)]
    pub frontmatter: FrontmatterSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillLayout {
    /// `<dir>/<skill-name>/SKILL.md`
    DirWithSkillMd,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDir {
    pub scope: Scope,
    pub path: String,
    /// Other providers read this same directory, so one write can serve several
    /// targets (F-5.2).
    #[serde(default)]
    pub shared: bool,
    /// Default write target for a single-provider deploy.
    #[serde(default)]
    pub canonical: bool,
    /// Read and reported, never written to.
    #[serde(default)]
    pub deprecated: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterSpec {
    /// Missing keys produce a warning. We never rewrite a user's frontmatter.
    #[serde(default)]
    pub required: Vec<String>,
}

impl Default for FrontmatterSpec {
    fn default() -> Self {
        FrontmatterSpec {
            required: vec!["name".into(), "description".into()],
        }
    }
}

// ---------------------------------------------------------------------------

/// serde needs a function for `#[serde(default = "...")]`; this is "default true".
fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Convenience accessors
// ---------------------------------------------------------------------------

impl ProviderSpec {
    pub fn file(&self, id: &str) -> Option<&FileSpec> {
        self.files.iter().find(|f| f.id == id)
    }

    pub fn supports_mcp(&self) -> bool {
        self.mcp.as_ref().is_some_and(|m| m.supported)
    }

    pub fn supports_skills(&self) -> bool {
        self.skills.as_ref().is_some_and(|s| s.supported)
    }

    /// Instruction files for a scope, in the order this harness reads them.
    pub fn instruction_files(&self, scope: Scope) -> Vec<&InstructionFile> {
        self.instructions.iter().filter(|f| f.scope == scope).collect()
    }

    /// Skill directories for a scope, in declared order. Deprecated dirs are
    /// included — discovery needs to find what is already there (F-2.5).
    pub fn skill_dirs(&self, scope: Scope) -> Vec<&SkillDir> {
        self.skills
            .iter()
            .flat_map(|s| s.dirs.iter())
            .filter(|d| d.scope == scope)
            .collect()
    }
}
