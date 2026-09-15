//! A format-agnostic value tree.
//!
//! TOML, JSONC and YAML all parse into this one shape, so a JSON Pointer from a
//! provider spec works identically against `config.toml`, `opencode.jsonc` and a
//! YAML frontmatter block. This is the "IR" in the compiler analogy.
//!
//! **Read-only.** Writing goes through format-specific, format-*preserving*
//! editors in M2 (`toml_edit`, the `jsonc-parser` CST) — never by serialising
//! this tree back out. Round-tripping through a plain value model is exactly the
//! bug we are differentiating against (RESEARCH §5).

use crate::error::{Error, Result};
use crate::pointer;
use crate::spec::Format;
use std::path::{Path, PathBuf};

/// An ordered string-keyed map.
///
/// A `Vec` of pairs rather than a `BTreeMap` so iteration follows file order,
/// which keeps CLI output looking like the user's file. Lookup is linear, but
/// these maps hold a handful of keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Map(Vec<(String, Value)>);

impl Map {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, Value)> for Map {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        Map(iter.into_iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Map(Map),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Strings from every element of an array, skipping non-strings.
    pub fn as_string_list(&self) -> Option<Vec<String>> {
        self.as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    }

    /// A `{key: "value"}` map as `(String, String)` pairs, dropping non-strings.
    /// Used for `env` and `headers`.
    pub fn as_string_map(&self) -> Option<Vec<(String, String)>> {
        self.as_map().map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
                .collect()
        })
    }

    /// Follow a JSON Pointer. `None` if any step is missing or not a map.
    ///
    /// Array indexing is not supported: no harness stores its server map inside
    /// an array, and silently mis-indexing would be worse than not resolving.
    pub fn at_pointer(&self, ptr: &str) -> Result<Option<&Value>> {
        let tokens = pointer::parse(ptr)?;
        let mut current = self;
        for token in tokens {
            match current.as_map().and_then(|m| m.get(&token)) {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    }

    /// Short type name, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Map(_) => "table",
        }
    }
}

// ---------------------------------------------------------------------------
// File state — F-2.4
// ---------------------------------------------------------------------------

/// The three distinct "nothing here" states, kept apart deliberately.
///
/// All three occur on the dev machine: `~/.gemini/settings.json` is absent,
/// `~/.gemini/config/mcp_config.json` is zero bytes, and `~/.claude.json` has
/// `mcpServers: null`. Conflating them loses real information — an empty file
/// means the harness is installed, an absent one may mean it is not.
#[derive(Debug, Clone)]
pub enum FileState {
    Absent,
    Empty,
    Parsed(Value),
    /// Parse failed. We report and refuse to write (NF-1.6), never guess.
    Invalid(String),
}

impl FileState {
    pub fn value(&self) -> Option<&Value> {
        match self {
            FileState::Parsed(v) => Some(v),
            _ => None,
        }
    }
    pub fn exists(&self) -> bool {
        !matches!(self, FileState::Absent)
    }
    pub fn label(&self) -> &'static str {
        match self {
            FileState::Absent => "absent",
            FileState::Empty => "empty",
            FileState::Parsed(_) => "ok",
            FileState::Invalid(_) => "INVALID",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub format: Format,
    pub state: FileState,
}

/// Read and parse one file. Missing and empty files are normal outcomes, not errors.
pub fn read_file(path: &Path, format: Format) -> ConfigFile {
    let state = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileState::Absent,
        Err(e) => FileState::Invalid(e.to_string()),
        Ok(text) if text.trim().is_empty() => FileState::Empty,
        Ok(text) => match parse(&text, format) {
            Ok(value) => FileState::Parsed(value),
            Err(e) => FileState::Invalid(e.to_string()),
        },
    };
    ConfigFile {
        path: path.to_path_buf(),
        format,
        state,
    }
}

pub fn parse(text: &str, format: Format) -> Result<Value> {
    match format {
        Format::Toml => parse_toml(text),
        Format::Jsonc => parse_jsonc(text),
        Format::Yaml => parse_yaml(text),
    }
}

fn parse_toml(text: &str) -> Result<Value> {
    // Deserialize as a `Table`, not a `Value`: in toml 1.x, `FromStr for Value`
    // parses a single standalone *value*, so a document starting with a
    // `[section]` header is read as an array and then fails with
    // "unexpected content, expected nothing".
    let parsed: toml::Table =
        toml::from_str(text).map_err(|e: toml::de::Error| Error::ConfigParse {
            path: PathBuf::new(),
            format: "TOML",
            reason: e.message().to_string(),
        })?;
    Ok(from_toml(toml::Value::Table(parsed)))
}

fn from_toml(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Int(i),
        toml::Value::Float(f) => Value::Float(f),
        toml::Value::Boolean(b) => Value::Bool(b),
        // A TOML datetime has no canonical-model equivalent; keep its text so it
        // survives display and comparison.
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(from_toml).collect()),
        toml::Value::Table(t) => {
            Value::Map(t.into_iter().map(|(k, v)| (k, from_toml(v))).collect())
        }
    }
}

/// Parsed with a JSONC parser even for `.json`, because real `.json` config files
/// contain `//` comments — `~/.copilot/config.json` does (RESEARCH §4).
fn parse_jsonc(text: &str) -> Result<Value> {
    let parsed = jsonc_parser::parse_to_serde_value(text, &Default::default()).map_err(|e| {
        Error::ConfigParse {
            path: PathBuf::new(),
            format: "JSONC",
            reason: e.to_string(),
        }
    })?;
    Ok(match parsed {
        Some(v) => from_json(v),
        None => Value::Null, // a file of only comments/whitespace
    })
}

fn from_json(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(a) => Value::Array(a.into_iter().map(from_json).collect()),
        serde_json::Value::Object(o) => {
            Value::Map(o.into_iter().map(|(k, v)| (k, from_json(v))).collect())
        }
    }
}

fn parse_yaml(text: &str) -> Result<Value> {
    use yaml_rust2::YamlLoader;
    let docs = YamlLoader::load_from_str(text).map_err(|e| Error::ConfigParse {
        path: PathBuf::new(),
        format: "YAML",
        reason: e.to_string(),
    })?;
    Ok(docs.into_iter().next().map(from_yaml).unwrap_or(Value::Null))
}

fn from_yaml(value: yaml_rust2::Yaml) -> Value {
    use yaml_rust2::Yaml;
    match value {
        Yaml::Real(s) => s.parse::<f64>().map(Value::Float).unwrap_or(Value::String(s)),
        Yaml::Integer(i) => Value::Int(i),
        Yaml::String(s) => Value::String(s),
        Yaml::Boolean(b) => Value::Bool(b),
        Yaml::Array(a) => Value::Array(a.into_iter().map(from_yaml).collect()),
        Yaml::Hash(h) => Value::Map(
            h.into_iter()
                .map(|(k, v)| {
                    // Non-string keys are legal YAML but meaningless in frontmatter;
                    // stringify rather than drop, so nothing vanishes silently.
                    let key = match k {
                        Yaml::String(s) => s,
                        other => format!("{other:?}"),
                    };
                    (key, from_yaml(v))
                })
                .collect(),
        ),
        Yaml::Null | Yaml::BadValue => Value::Null,
        Yaml::Alias(_) => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_with_nested_tables() {
        let v = parse(
            r#"
[mcp_servers.node_repl]
command = 'C:\Users\User\node.exe'
args = []
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
CODEX_HOME = 'C:\Users\User\.codex'
"#,
            Format::Toml,
        )
        .unwrap();

        let server = v.at_pointer("/mcp_servers/node_repl").unwrap().unwrap();
        assert_eq!(
            server.as_map().unwrap().get("command").unwrap().as_str(),
            Some(r"C:\Users\User\node.exe")
        );
        // Harness-specific field that must survive round-tripping later.
        assert_eq!(
            server.as_map().unwrap().get("startup_timeout_sec"),
            Some(&Value::Int(120))
        );
        let env = server.at_pointer("/env").unwrap().unwrap();
        assert_eq!(env.as_string_map().unwrap().len(), 1);
    }

    /// RESEARCH §4: a `.json` file containing `//` comments.
    #[test]
    fn parses_json_containing_comments() {
        let v = parse(
            r#"
// User settings belong in settings.json.
// This file is managed automatically.
{
  "firstLaunchAt": "2026-07-24T16:00:51.678Z"
}
"#,
            Format::Jsonc,
        )
        .unwrap();
        assert!(v.as_map().unwrap().contains_key("firstLaunchAt"));
    }

    /// The OpenCode shape: merged argv array, `environment` not `env`.
    #[test]
    fn parses_opencode_mcp_block() {
        let v = parse(
            r#"{
  "mcp": {
    "obsidian": {
      "type": "local",
      "command": ["uvx", "mcp-obsidian"],
      "enabled": true,
      "environment": { "OBSIDIAN_HOST": "127.0.0.1" }
    }
  }
}"#,
            Format::Jsonc,
        )
        .unwrap();
        let server = v.at_pointer("/mcp/obsidian").unwrap().unwrap();
        let map = server.as_map().unwrap();
        assert_eq!(map.get("type").unwrap().as_str(), Some("local"));
        assert_eq!(
            map.get("command").unwrap().as_string_list().unwrap(),
            vec!["uvx", "mcp-obsidian"]
        );
        assert!(map.contains_key("environment"));
    }

    /// Claude Code's project-namespaced pointer, with a Windows path as the key.
    #[test]
    fn resolves_pointer_through_escaped_windows_path() {
        let v = parse(
            r#"{
  "projects": {
    "C:/Users/User": {
      "mcpServers": { "notion": { "type": "http", "url": "https://mcp.notion.com/mcp" } }
    }
  }
}"#,
            Format::Jsonc,
        )
        .unwrap();

        let escaped = pointer::escape_token("C:/Users/User");
        let servers = v
            .at_pointer(&format!("/projects/{escaped}/mcpServers"))
            .unwrap()
            .unwrap();
        assert!(servers.as_map().unwrap().contains_key("notion"));

        // Without escaping, the same logical path resolves to nothing.
        assert!(
            v.at_pointer("/projects/C:/Users/User/mcpServers")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn missing_pointer_is_none_not_error() {
        let v = parse("{}", Format::Jsonc).unwrap();
        assert!(v.at_pointer("/mcpServers").unwrap().is_none());
    }

    /// F-2.4: `"mcpServers": null` is a real state in ~/.claude.json and must not
    /// look the same as an absent key.
    #[test]
    fn null_value_is_distinct_from_missing() {
        let v = parse(r#"{"mcpServers": null}"#, Format::Jsonc).unwrap();
        assert_eq!(v.at_pointer("/mcpServers").unwrap(), Some(&Value::Null));
        assert_eq!(v.at_pointer("/other").unwrap(), None);
    }

    #[test]
    fn parses_yaml_frontmatter_shape() {
        let v = parse(
            r#"
name: skill-creator
description: Guide for creating effective skills.
metadata:
  short-description: Create or update a skill
"#,
            Format::Yaml,
        )
        .unwrap();
        let m = v.as_map().unwrap();
        assert_eq!(m.get("name").unwrap().as_str(), Some("skill-creator"));
        // Nested frontmatter must survive — real skills use it.
        assert!(m.get("metadata").unwrap().as_map().is_some());
    }

    #[test]
    fn map_preserves_insertion_order() {
        let v = parse(r#"{"z": 1, "a": 2, "m": 3}"#, Format::Jsonc).unwrap();
        let keys: Vec<&str> = v.as_map().unwrap().keys().collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn invalid_toml_is_reported_not_guessed() {
        assert!(parse("this is not valid TOML [[[", Format::Toml).is_err());
    }
}
