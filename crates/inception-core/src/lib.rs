//! project-inception engine.
//!
//! A translation engine for agent-harness configuration. Harness support is
//! defined by data — `providers/*.provider.toml` — not by code. No harness name
//! appears anywhere in this crate outside test fixtures (F-1.1).
//!
//! Pipeline:
//!
//! ```text
//! provider spec  ->  [ read ]  ->  canonical model  ->  [ write ]  ->  provider config
//!  (TOML data)                    McpServer / Skill              surgical, format-preserving
//! ```
//!
//! This crate has no GUI and no network dependency (NF-4.2, NF-5.1).

pub mod diff;
pub mod discover;
pub mod doctor;
pub mod editor;
pub mod emit;
pub mod error;
pub mod inventory;
pub mod mcp;
pub mod paths;
pub mod plan;
pub mod pointer;
pub mod projects;
pub mod registry;
pub mod scope;
pub mod secrets;
pub mod settings;
pub mod skills;
pub mod spec;
pub mod tree;
pub mod when;
pub mod write;

pub use discover::{DetectedProvider, detect};
pub use editor::Editor;
pub use error::{Error, Result};
pub use paths::PathContext;
pub use registry::Registry;
pub use mcp::{McpServer, Secret};
pub use skills::{Skill, SkillEntry, SkillState};
pub use settings::Settings;
pub use spec::ProviderSpec;
pub use tree::{ConfigFile, FileState, Value};
