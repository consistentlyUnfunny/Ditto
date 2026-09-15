//! Reading MCP servers through a provider spec (F-3.4).
//!
//! This is the "parse" half of the compiler analogy: a harness-shaped object
//! becomes an [`McpServer`]. The "emit" half arrives in M3 and must be its exact
//! inverse.
//!
//! Everything harness-specific comes from the spec. Nothing in this file knows
//! that OpenCode calls it `environment` or that Copilot's map key is `servers`.

use crate::spec::{FieldSpec, Form, McpSpec, Transport, TransportSpec, WriteRule};
use crate::tree::{Map, Value};

/// A credential value. In M1 everything is `Literal`; promotion to `Ref` and
/// keychain storage land with F-7 in M3.
#[derive(Debug, Clone, PartialEq)]
pub enum Secret {
    Literal(String),
    Ref(String),
}

impl Secret {
    /// Never render the value itself (F-7.4).
    pub fn redacted(&self) -> String {
        match self {
            Secret::Ref(name) => format!("${{secret:{name}}}"),
            Secret::Literal(v) if v.is_empty() => String::new(),
            Secret::Literal(v) => format!("<{} chars redacted>", v.chars().count()),
        }
    }

    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Secret::Literal(v) => Some(v),
            Secret::Ref(_) => None,
        }
    }
}

/// The canonical form of an MCP server, independent of any harness.
#[derive(Debug, Clone)]
pub struct McpServer {
    pub name: String,
    pub transport: Transport,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, Secret)>,
    pub url: Option<String>,
    pub headers: Vec<(String, Secret)>,
    pub enabled: bool,
    /// Keys this harness carries that the canonical model does not describe:
    /// `startup_timeout_sec`, `trust`, `timeout`, and so on. Preserved verbatim
    /// so a round-trip cannot corrupt the user's configuration
    /// (PROVIDER-SPEC §6).
    pub extra: Map,
    /// Which provider this was read from, so `extra` can be restored only to its
    /// own harness.
    pub source_provider: String,
}

impl McpServer {
    /// Env and header names whose values look like credentials (F-7.1, NF-4.4).
    ///
    /// Deliberately name-based, not value-based: matching on value shape would
    /// mean inspecting — and potentially logging — the secrets themselves.
    pub fn secret_like(&self) -> Vec<&str> {
        self.env
            .iter()
            .chain(self.headers.iter())
            .filter(|(k, v)| looks_secret(k) && v.as_literal().is_some_and(|s| !s.is_empty()))
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Command plus args as one displayable string.
    pub fn command_line(&self) -> String {
        if self.command.is_empty() {
            return self.url.clone().unwrap_or_default();
        }
        std::iter::once(self.command.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn looks_secret(key: &str) -> bool {
    const MARKERS: &[&str] = &[
        "key", "token", "secret", "password", "passwd", "credential", "auth", "bearer",
    ];
    let lower = key.to_ascii_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read every server from the map a pointer resolved to.
///
/// `servers` is the `name -> object` map. A non-map value (including `null`,
/// which is what `~/.claude.json` holds when no user-scope servers exist) yields
/// an empty list rather than an error.
pub fn read_servers(spec: &McpSpec, provider_id: &str, servers: &Value) -> Vec<McpServer> {
    let Some(map) = servers.as_map() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, object)| read_server(spec, provider_id, name, object))
        .collect()
}

fn read_server(
    spec: &McpSpec,
    provider_id: &str,
    name: &str,
    object: &Value,
) -> Option<McpServer> {
    let map = object.as_map()?;

    let transport = read_transport(&spec.transport, map);

    // Track which keys the spec accounts for; everything else is `extra`.
    let mut claimed: Vec<&str> = transport_keys(&spec.transport);

    let mut command = String::new();
    let mut args = Vec::new();
    let mut url = None;
    let mut env = Vec::new();
    let mut headers = Vec::new();
    let mut enabled = true;

    for (canonical, field) in &spec.fields {
        claimed.push(field.key.as_str());
        let raw = map.get(&field.key);

        match canonical.as_str() {
            "command" => {
                if field.form == Some(Form::ArgvArray) {
                    // One array holds the command and its arguments:
                    // ["uvx", "mcp-obsidian"] -> command "uvx", args ["mcp-obsidian"].
                    if let Some(list) = raw.and_then(Value::as_string_list)
                        && let Some((head, tail)) = list.split_first()
                    {
                        command = head.clone();
                        args = tail.to_vec();
                    }
                } else if let Some(s) = raw.and_then(Value::as_str) {
                    command = s.to_string();
                }
            }
            "args" => {
                if let Some(list) = raw.and_then(Value::as_string_list) {
                    args = list;
                }
            }
            "url" => url = raw.and_then(Value::as_str).map(str::to_string),
            "env" => env = read_secret_map(raw),
            "headers" => headers = read_secret_map(raw),
            "enabled" => {
                enabled = raw
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| default_bool(field, true));
            }
            // An unrecognised canonical name in a spec is ignored rather than
            // fatal, so a spec written for a newer build still mostly works.
            _ => {}
        }
    }

    // For key-presence harnesses the url lives under whichever key was present.
    if url.is_none() {
        if let TransportSpec::KeyPresence { keys, .. } = &spec.transport {
            for key in keys.values() {
                if let Some(v) = map.get(key).and_then(Value::as_str) {
                    url = Some(v.to_string());
                    break;
                }
            }
        }
    }

    let extra: Map = map
        .iter()
        .filter(|(k, _)| !claimed.contains(k))
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();

    Some(McpServer {
        name: name.to_string(),
        transport,
        command,
        args,
        env,
        url,
        headers,
        enabled,
        extra,
        source_provider: provider_id.to_string(),
    })
}

fn default_bool(field: &FieldSpec, fallback: bool) -> bool {
    field
        .default
        .as_ref()
        .and_then(|v| v.as_bool())
        .unwrap_or(fallback)
}

fn read_secret_map(raw: Option<&Value>) -> Vec<(String, Secret)> {
    raw.and_then(Value::as_string_map)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, Secret::Literal(v)))
        .collect()
}

/// Apply the spec's transport strategy (PROVIDER-SPEC §4.1).
fn read_transport(spec: &TransportSpec, map: &Map) -> Transport {
    match spec {
        TransportSpec::TypeValue {
            key, absent, read, ..
        } => match map.get(key).and_then(Value::as_str) {
            // An unrecognised value falls back to `absent` rather than guessing.
            Some(value) => read.get(value).copied().unwrap_or(*absent),
            None => *absent,
        },
        TransportSpec::KeyPresence { absent, keys } => keys
            .iter()
            .find(|(_, key)| map.contains_key(key))
            .map(|(transport, _)| *transport)
            .unwrap_or(*absent),
        TransportSpec::UrlPresence { key, remote } => {
            if map.contains_key(key) {
                *remote
            } else {
                Transport::Stdio
            }
        }
    }
}

/// Keys the transport strategy itself consumes, so they are not double-counted
/// as `extra`.
fn transport_keys(spec: &TransportSpec) -> Vec<&str> {
    match spec {
        TransportSpec::TypeValue { key, .. } => vec![key.as_str()],
        TransportSpec::KeyPresence { keys, .. } => keys.values().map(String::as_str).collect(),
        TransportSpec::UrlPresence { key, .. } => vec![key.as_str()],
    }
}

/// Canonical -> harness value for emission. Used by M3; defined here so the
/// read and write mappings stay next to each other.
pub fn write_transport_value(spec: &TransportSpec, transport: Transport) -> Option<String> {
    match spec {
        TransportSpec::TypeValue { write, .. } => match write.get(&transport) {
            Some(WriteRule::Value(v)) => Some(v.clone()),
            Some(WriteRule::Omit { omit }) if *omit => None,
            Some(WriteRule::Omit { .. }) | None => None,
        },
        TransportSpec::KeyPresence { .. } | TransportSpec::UrlPresence { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Format;
    use crate::tree;

    fn spec_for(id: &str) -> McpSpec {
        crate::Registry::builtin()
            .get(id)
            .unwrap()
            .mcp
            .clone()
            .unwrap()
    }

    fn read(id: &str, text: &str, pointer: &str, format: Format) -> Vec<McpServer> {
        let doc = tree::parse(text, format).unwrap();
        let servers = doc.at_pointer(pointer).unwrap().unwrap();
        read_servers(&spec_for(id), id, servers)
    }

    /// The real entry from ~/.config/opencode/opencode.jsonc — merged argv array,
    /// `environment` rather than `env`, `type: "local"` meaning stdio.
    #[test]
    fn reads_opencode_obsidian_server() {
        let servers = read(
            "opencode",
            r#"{
  "mcp": {
    "obsidian": {
      "type": "local",
      "command": ["uvx", "mcp-obsidian"],
      "enabled": true,
      "environment": {
        "OBSIDIAN_API_KEY": "secret-value-here",
        "OBSIDIAN_HOST": "127.0.0.1"
      }
    }
  }
}"#,
            "/mcp",
            Format::Jsonc,
        );

        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.name, "obsidian");
        assert_eq!(s.transport, Transport::Stdio); // "local" -> stdio
        assert_eq!(s.command, "uvx"); // argv array split
        assert_eq!(s.args, vec!["mcp-obsidian"]);
        assert!(s.enabled);
        assert_eq!(s.env.len(), 2);
        assert_eq!(s.secret_like(), vec!["OBSIDIAN_API_KEY"]);
        assert!(s.extra.is_empty());
    }

    /// The real entry from ~/.codex/config.toml — TOML, env as a sub-table, and
    /// a harness-specific field that must land in `extra`.
    #[test]
    fn reads_codex_server_and_preserves_unknown_fields() {
        let servers = read(
            "codex",
            r#"
[mcp_servers.node_repl]
command = 'C:\Users\User\node_repl.exe'
args = []
startup_timeout_sec = 120

[mcp_servers.node_repl.env]
CODEX_HOME = 'C:\Users\User\.codex'
"#,
            "/mcp_servers",
            Format::Toml,
        );

        let s = &servers[0];
        assert_eq!(s.transport, Transport::Stdio); // no url -> stdio
        assert_eq!(s.command, r"C:\Users\User\node_repl.exe");
        assert!(s.args.is_empty());
        assert_eq!(s.env.len(), 1);
        // PROVIDER-SPEC §6: dropping this would corrupt the user's config.
        assert_eq!(
            s.extra.get("startup_timeout_sec"),
            Some(&crate::Value::Int(120))
        );
    }

    /// The real entry from ~/.claude.json, reached through a project-namespaced
    /// pointer with a Windows path as the key.
    #[test]
    fn reads_claude_remote_server_via_dynamic_pointer() {
        let escaped = crate::pointer::escape_token("C:/Users/User");
        let servers = read(
            "claude-code",
            r#"{
  "projects": {
    "C:/Users/User": {
      "mcpServers": {
        "notion": { "type": "http", "url": "https://mcp.notion.com/mcp" }
      }
    }
  }
}"#,
            &format!("/projects/{escaped}/mcpServers"),
            Format::Jsonc,
        );

        let s = &servers[0];
        assert_eq!(s.name, "notion");
        assert_eq!(s.transport, Transport::Http);
        assert_eq!(s.url.as_deref(), Some("https://mcp.notion.com/mcp"));
        assert!(s.command.is_empty());
    }

    /// Claude accepts both spellings on read, but must emit only `http`.
    #[test]
    fn streamable_http_reads_as_http_and_writes_as_http() {
        let servers = read(
            "claude-code",
            r#"{"mcpServers": {"x": {"type": "streamable-http", "url": "https://e.com"}}}"#,
            "/mcpServers",
            Format::Jsonc,
        );
        assert_eq!(servers[0].transport, Transport::Http);

        let spec = spec_for("claude-code");
        assert_eq!(
            write_transport_value(&spec.transport, Transport::Http).as_deref(),
            Some("http")
        );
        // stdio writes no `type` key at all.
        assert_eq!(write_transport_value(&spec.transport, Transport::Stdio), None);
    }

    /// Gemini's key-name discriminator: `url` = SSE, `httpUrl` = streamable HTTP.
    /// This is the case that a value-mapping table cannot express.
    #[test]
    fn gemini_discriminates_transport_by_key_name() {
        let sse = read(
            "gemini-cli",
            r#"{"mcpServers": {"a": {"url": "https://e.com/sse"}}}"#,
            "/mcpServers",
            Format::Jsonc,
        );
        assert_eq!(sse[0].transport, Transport::Sse);
        assert_eq!(sse[0].url.as_deref(), Some("https://e.com/sse"));

        let http = read(
            "gemini-cli",
            r#"{"mcpServers": {"a": {"httpUrl": "https://e.com/mcp"}}}"#,
            "/mcpServers",
            Format::Jsonc,
        );
        assert_eq!(http[0].transport, Transport::Http);
        assert_eq!(http[0].url.as_deref(), Some("https://e.com/mcp"));

        let stdio = read(
            "gemini-cli",
            r#"{"mcpServers": {"a": {"command": "node", "args": ["s.js"]}}}"#,
            "/mcpServers",
            Format::Jsonc,
        );
        assert_eq!(stdio[0].transport, Transport::Stdio);
    }

    /// Gemini's own extras must survive.
    #[test]
    fn gemini_extras_are_preserved() {
        let servers = read(
            "gemini-cli",
            r#"{"mcpServers": {"a": {"command": "node", "timeout": 600000, "trust": true}}}"#,
            "/mcpServers",
            Format::Jsonc,
        );
        assert_eq!(servers[0].extra.get("timeout"), Some(&crate::Value::Int(600000)));
        assert_eq!(servers[0].extra.get("trust"), Some(&crate::Value::Bool(true)));
    }

    /// Copilot's map key is `servers`, not `mcpServers` — an easy thing to get
    /// wrong when copying from the other JSON harnesses.
    #[test]
    fn copilot_uses_the_servers_key() {
        let servers = read(
            "copilot-cli",
            r#"{"servers": {"a": {"type": "local", "command": "node"}}}"#,
            "/servers",
            Format::Jsonc,
        );
        assert_eq!(servers[0].transport, Transport::Stdio);
        assert_eq!(servers[0].command, "node");
    }

    /// F-2.4: `"mcpServers": null` must read as "no servers", not crash.
    #[test]
    fn null_server_map_reads_as_empty() {
        let doc = tree::parse(r#"{"mcpServers": null}"#, Format::Jsonc).unwrap();
        let servers = doc.at_pointer("/mcpServers").unwrap().unwrap();
        assert!(read_servers(&spec_for("claude-code"), "claude-code", servers).is_empty());
    }

    #[test]
    fn unknown_transport_value_falls_back_rather_than_guessing() {
        let servers = read(
            "opencode",
            r#"{"mcp": {"a": {"type": "quantum", "command": ["x"]}}}"#,
            "/mcp",
            Format::Jsonc,
        );
        assert_eq!(servers[0].transport, Transport::Stdio); // `absent` default
    }

    #[test]
    fn secrets_are_never_rendered() {
        let s = Secret::Literal("super-secret-value".into());
        let shown = s.redacted();
        assert!(!shown.contains("super-secret"));
        assert!(shown.contains("18 chars"));
    }

    #[test]
    fn secret_detection_is_name_based() {
        assert!(looks_secret("OBSIDIAN_API_KEY"));
        assert!(looks_secret("Authorization"));
        assert!(looks_secret("github_token"));
        assert!(!looks_secret("OBSIDIAN_HOST"));
        assert!(!looks_secret("PORT"));
    }
}
