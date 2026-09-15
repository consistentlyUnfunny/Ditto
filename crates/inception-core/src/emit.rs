//! Canonical → provider shape (T-3.1). The exact inverse of [`crate::mcp`].
//!
//! Every asymmetry the reader absorbed has to be put back: the argv array
//! re-merged, `env` renamed to `environment`, the transport re-encoded by
//! whichever of the three mechanisms this harness uses.
//!
//! Two rules that make the difference between a useful tool and a destructive
//! one:
//!
//!   - **Never downgrade silently** (F-5.4). Deploying a `ws` server to Codex is
//!     an error with an explanation, not a quietly broken entry.
//!   - **Never drop anything quietly** (F-5.5). Fields that cannot cross are
//!     reported, per target, in the preview.

use crate::error::{Error, Result};
use crate::mcp::McpServer;
use crate::secrets::SecretStore;
use crate::spec::{Form, McpSpec, Transport, TransportSpec, WriteRule};
use crate::tree::{Map, Value};

/// One server rendered for one provider.
#[derive(Debug)]
pub struct Emitted {
    /// The object to write under the server's name.
    pub value: Value,
    /// Harness-specific keys that could not cross (PROVIDER-SPEC §6 rule 2).
    pub dropped: Vec<String>,
    /// Non-fatal fidelity losses worth telling the user about.
    pub warnings: Vec<String>,
}

/// Render `server` into `target`'s shape.
///
/// Fails rather than producing a broken entry when the target cannot express the
/// server's transport.
pub fn server(
    spec: &McpSpec,
    target_id: &str,
    server: &McpServer,
    secrets: &dyn SecretStore,
) -> Result<Emitted> {
    let mut warnings = Vec::new();

    // --- capability gate (F-5.4) -----------------------------------------
    if !spec.capabilities.transports.contains(&server.transport) {
        return Err(Error::Unsupported {
            provider: target_id.to_string(),
            what: format!("transport '{}'", server.transport.as_str()),
            supported: spec
                .capabilities
                .transports
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    if !server.env.is_empty() && !spec.capabilities.env {
        return Err(Error::Unsupported {
            provider: target_id.to_string(),
            what: "environment variables".to_string(),
            supported: "none".to_string(),
        });
    }
    if !server.headers.is_empty() && !spec.capabilities.headers {
        return Err(Error::Unsupported {
            provider: target_id.to_string(),
            what: "HTTP headers".to_string(),
            supported: "none".to_string(),
        });
    }

    // Key order is chosen for readability, since we are writing into a file a
    // human maintains: discriminator first, then how to reach the server, then
    // its environment, then anything harness-specific.
    let mut out = Map::default();

    // --- transport --------------------------------------------------------
    match &spec.transport {
        TransportSpec::TypeValue { key, write, .. } => {
            match write.get(&server.transport) {
                Some(WriteRule::Value(v)) => out.insert(key.clone(), Value::String(v.clone())),
                // `{ omit = true }` — stdio is expressed by writing no key.
                Some(WriteRule::Omit { .. }) | None => {}
            }
            // Lossy mappings: OpenCode collapses sse and http into "remote", so
            // a round-trip back will report http. Say so rather than hide it.
            if let Some(WriteRule::Value(v)) = write.get(&server.transport) {
                let collides = write.iter().any(|(t, rule)| {
                    *t != server.transport && matches!(rule, WriteRule::Value(other) if other == v)
                });
                if collides {
                    warnings.push(format!(
                        "{target_id} represents several transports as '{v}', so '{}' will read \
                         back as a different transport",
                        server.transport.as_str()
                    ));
                }
            }
        }
        TransportSpec::KeyPresence { keys, .. } => {
            // Gemini: the *key name* carries the transport.
            if let (Some(url), Some(key)) = (&server.url, keys.get(&server.transport)) {
                out.insert(key.clone(), Value::String(url.clone()));
            }
        }
        TransportSpec::UrlPresence { key, .. } => {
            if let Some(url) = &server.url {
                out.insert(key.clone(), Value::String(url.clone()));
            }
        }
    }

    // --- mapped fields ----------------------------------------------------
    let field = |name: &str| spec.fields.get(name);

    if server.transport == Transport::Stdio {
        if let Some(command) = field("command") {
            if command.form == Some(Form::ArgvArray) {
                // Re-merge into one array: ["uvx", "mcp-obsidian"].
                let mut argv = vec![Value::String(server.command.clone())];
                argv.extend(server.args.iter().cloned().map(Value::String));
                out.insert(command.key.clone(), Value::Array(argv));
            } else {
                out.insert(command.key.clone(), Value::String(server.command.clone()));
                if let Some(args) = field("args")
                    && !server.args.is_empty()
                {
                    out.insert(
                        args.key.clone(),
                        Value::Array(server.args.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
        }
    } else if let Some(url_field) = field("url")
        && let Some(url) = &server.url
        && !out.contains_key(&url_field.key)
    {
        // key-presence harnesses already wrote the url above.
        out.insert(url_field.key.clone(), Value::String(url.clone()));
    }

    if let Some(headers) = field("headers")
        && !server.headers.is_empty()
    {
        out.insert(headers.key.clone(), secret_map(&server.headers, secrets)?);
    }
    if let Some(env) = field("env")
        && !server.env.is_empty()
    {
        out.insert(env.key.clone(), secret_map(&server.env, secrets)?);
    }

    // Deploying a credential means writing it, in cleartext, into one more file.
    //
    // There is no way around this: an MCP server reads its environment from the
    // harness's own config, so if the harness has no indirection mechanism the
    // literal has to be there. The keychain stops *us* holding copies; it cannot
    // stop the harness needing one. Saying so is the only honest option —
    // silently multiplying a secret across five files is exactly the behaviour
    // this tool exists to improve on.
    let credentials = server.secret_like();
    if !credentials.is_empty() && !same_provider_hint(server, target_id) {
        warnings.push(format!(
            "writes {} in cleartext to {target_id}'s config — this is now one more \
             copy on disk ({} supports no credential indirection)",
            credentials.join(", "),
            target_id
        ));
    }
    if let Some(enabled) = field("enabled") {
        let default = enabled
            .default
            .as_ref()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        // Only write the flag when it differs from the harness default, so we do
        // not add noise to a file that did not have it.
        if server.enabled != default {
            out.insert(enabled.key.clone(), Value::Bool(server.enabled));
        }
    }

    // --- unmodelled fields (PROVIDER-SPEC §6) -----------------------------
    let mut dropped = Vec::new();
    let same_provider = server.source_provider == target_id;
    for (key, value) in server.extra.iter() {
        if same_provider {
            // Rule 1: restore this harness's own fields verbatim.
            out.insert(key.to_string(), value.clone());
        } else {
            // Rule 2: another harness's fields are meaningless here.
            dropped.push(key.to_string());
        }
    }

    // Losses the canonical model itself cannot carry across.
    if !same_provider {
        if server.transport == Transport::Stdio
            && !server.args.is_empty()
            && field("args").is_none()
            && field("command").and_then(|c| c.form) != Some(Form::ArgvArray)
        {
            warnings.push(format!("{target_id} has no field for command arguments"));
        }
        if !server.headers.is_empty() && field("headers").is_none() {
            warnings.push(format!("{target_id} has no field for HTTP headers"));
        }
    }

    Ok(Emitted {
        value: Value::Map(out),
        dropped,
        warnings,
    })
}

/// A copy of an emitted entry with credential values masked, for display (F-7.4).
///
/// **Why this exists.** The emitter resolves secrets to their real values because
/// that is what must be written to disk. Rendering that same value into a diff
/// preview prints the credential to the terminal — which is exactly what happened
/// the first time this was wired up, with a real API key.
///
/// Masking is key-aware rather than blanket: `OBSIDIAN_API_KEY` is hidden,
/// `OBSIDIAN_HOST` is not, because seeing the non-secret half of the environment
/// is the point of a preview.
pub fn redacted_copy(spec: &McpSpec, value: &Value) -> Value {
    let Value::Map(map) = value else {
        return value.clone();
    };
    let sensitive: Vec<&str> = ["env", "headers"]
        .iter()
        .filter_map(|name| spec.fields.get(*name))
        .map(|f| f.key.as_str())
        .collect();

    let mut out = Map::default();
    for (key, entry) in map.iter() {
        if sensitive.contains(&key)
            && let Value::Map(inner) = entry
        {
            let mut masked = Map::default();
            for (name, v) in inner.iter() {
                if crate::mcp::looks_secret(name) {
                    let len = v.as_str().map(|s| s.chars().count()).unwrap_or(0);
                    masked.insert(name, Value::String(format!("<{len} chars redacted>")));
                } else {
                    masked.insert(name, v.clone());
                }
            }
            out.insert(key, Value::Map(masked));
        } else {
            out.insert(key, entry.clone());
        }
    }
    Value::Map(out)
}

/// Whether this emit is writing back into the file the server came from, in
/// which case the credential is already there and no new copy is created.
fn same_provider_hint(server: &McpServer, target_id: &str) -> bool {
    server.source_provider == target_id
}

fn secret_map(pairs: &[(String, crate::mcp::Secret)], secrets: &dyn SecretStore) -> Result<Value> {
    let mut map = Map::default();
    for (key, secret) in pairs {
        map.insert(key.clone(), Value::String(secrets.resolve(secret)?));
    }
    Ok(Value::Map(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::Secret;
    use crate::secrets::LiteralsOnly;
    use crate::spec::Format;
    use crate::{Registry, tree};

    fn spec_for(id: &str) -> McpSpec {
        Registry::builtin().get(id).unwrap().mcp.clone().unwrap()
    }

    /// Read a server from one provider, emit into another — the full journey.
    fn translate(from: &str, text: &str, pointer: &str, format: Format, to: &str) -> Emitted {
        let doc = tree::parse(text, format).unwrap();
        let servers = doc.at_pointer(pointer).unwrap().unwrap();
        let read = crate::mcp::read_servers(&spec_for(from), from, servers);
        server(&spec_for(to), to, &read[0], &LiteralsOnly).unwrap()
    }

    const OPENCODE_OBSIDIAN: &str = r#"{
      "mcp": { "obsidian": {
        "type": "local",
        "command": ["uvx", "mcp-obsidian"],
        "environment": { "OBSIDIAN_HOST": "127.0.0.1" }
      }}
    }"#;

    /// The MVP acceptance journey: OpenCode's merged argv array must come apart
    /// and land as Codex's separate `command` + `args`.
    #[test]
    fn opencode_to_codex_splits_the_argv_array() {
        let out = translate("opencode", OPENCODE_OBSIDIAN, "/mcp", Format::Jsonc, "codex");
        let m = out.value.as_map().unwrap();

        assert_eq!(m.get("command").unwrap().as_str(), Some("uvx"));
        assert_eq!(
            m.get("args").unwrap().as_string_list().unwrap(),
            vec!["mcp-obsidian"]
        );
        // Codex calls it `env`, not `environment`.
        assert!(m.contains_key("env"));
        assert!(!m.contains_key("environment"));
        // stdio is implicit in Codex — no discriminator key at all.
        assert!(!m.contains_key("type"));
        assert!(out.dropped.is_empty());
    }

    #[test]
    fn opencode_to_claude_writes_no_type_for_stdio() {
        let out = translate("opencode", OPENCODE_OBSIDIAN, "/mcp", Format::Jsonc, "claude-code");
        let m = out.value.as_map().unwrap();
        assert_eq!(m.get("command").unwrap().as_str(), Some("uvx"));
        assert!(m.contains_key("args"));
        assert!(m.contains_key("env"));
        // Claude Code omits `type` for stdio.
        assert!(!m.contains_key("type"));
    }

    /// Round-tripping back into OpenCode must re-merge the array.
    #[test]
    fn codex_to_opencode_remerges_the_argv_array() {
        let out = translate(
            "codex",
            "[mcp_servers.x]\ncommand = 'uvx'\nargs = ['mcp-obsidian']\n",
            "/mcp_servers",
            Format::Toml,
            "opencode",
        );
        let m = out.value.as_map().unwrap();
        assert_eq!(
            m.get("command").unwrap().as_string_list().unwrap(),
            vec!["uvx", "mcp-obsidian"]
        );
        assert_eq!(m.get("type").unwrap().as_str(), Some("local"));
        assert!(!m.contains_key("args"), "args should be folded into command");
    }

    /// Gemini encodes the transport in the key name.
    #[test]
    fn remote_server_to_gemini_uses_the_right_url_key() {
        let http = translate(
            "claude-code",
            r#"{"mcpServers": {"a": {"type": "http", "url": "https://e.com/mcp"}}}"#,
            "/mcpServers",
            Format::Jsonc,
            "gemini-cli",
        );
        let m = http.value.as_map().unwrap();
        assert_eq!(m.get("httpUrl").unwrap().as_str(), Some("https://e.com/mcp"));
        assert!(!m.contains_key("url"));

        let sse = translate(
            "claude-code",
            r#"{"mcpServers": {"a": {"type": "sse", "url": "https://e.com/sse"}}}"#,
            "/mcpServers",
            Format::Jsonc,
            "gemini-cli",
        );
        let m = sse.value.as_map().unwrap();
        assert_eq!(m.get("url").unwrap().as_str(), Some("https://e.com/sse"));
        assert!(!m.contains_key("httpUrl"));
    }

    /// Claude Code errors on a url with no type, so `type` is always written.
    #[test]
    fn remote_server_to_claude_always_writes_type() {
        let out = translate(
            "codex",
            "[mcp_servers.a]\nurl = 'https://e.com/mcp'\n",
            "/mcp_servers",
            Format::Toml,
            "claude-code",
        );
        let m = out.value.as_map().unwrap();
        assert_eq!(m.get("type").unwrap().as_str(), Some("http"));
        assert_eq!(m.get("url").unwrap().as_str(), Some("https://e.com/mcp"));
    }

    /// F-5.4: refuse rather than write something broken.
    #[test]
    fn websocket_server_is_refused_by_codex() {
        let doc = tree::parse(
            r#"{"mcpServers": {"a": {"type": "ws", "url": "wss://e.com"}}}"#,
            Format::Jsonc,
        )
        .unwrap();
        let read = crate::mcp::read_servers(
            &spec_for("claude-code"),
            "claude-code",
            doc.at_pointer("/mcpServers").unwrap().unwrap(),
        );
        assert_eq!(read[0].transport, Transport::Ws);

        let err = server(&spec_for("codex"), "codex", &read[0], &LiteralsOnly).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("ws"), "got: {text}");
        assert!(text.contains("stdio"), "error should list what IS supported: {text}");
    }

    /// PROVIDER-SPEC §6 rule 1: a harness's own extras are restored verbatim.
    #[test]
    fn same_provider_extras_are_restored() {
        let out = translate(
            "codex",
            "[mcp_servers.x]\ncommand = 'srv'\nstartup_timeout_sec = 120\n",
            "/mcp_servers",
            Format::Toml,
            "codex",
        );
        assert_eq!(
            out.value.as_map().unwrap().get("startup_timeout_sec"),
            Some(&Value::Int(120))
        );
        assert!(out.dropped.is_empty());
    }

    /// Rule 2: another harness's extras are dropped — and *reported* (F-5.5).
    #[test]
    fn cross_provider_extras_are_dropped_and_reported() {
        let out = translate(
            "codex",
            "[mcp_servers.x]\ncommand = 'srv'\nstartup_timeout_sec = 120\n",
            "/mcp_servers",
            Format::Toml,
            "opencode",
        );
        assert!(!out.value.as_map().unwrap().contains_key("startup_timeout_sec"));
        assert_eq!(out.dropped, vec!["startup_timeout_sec"]);
    }

    /// OpenCode has one "remote" bucket for both sse and http, so the user should
    /// be told the transport will not survive a round trip.
    #[test]
    fn lossy_transport_mapping_warns() {
        let out = translate(
            "claude-code",
            r#"{"mcpServers": {"a": {"type": "sse", "url": "https://e.com"}}}"#,
            "/mcpServers",
            Format::Jsonc,
            "opencode",
        );
        assert_eq!(out.value.as_map().unwrap().get("type").unwrap().as_str(), Some("remote"));
        assert!(
            out.warnings.iter().any(|w| w.contains("read back")),
            "expected a lossy-mapping warning, got {:?}",
            out.warnings
        );
    }

    /// Don't add an `enabled: true` that the source file never had.
    #[test]
    fn default_valued_flags_are_not_written() {
        let out = translate(
            "codex",
            "[mcp_servers.x]\ncommand = 'srv'\n",
            "/mcp_servers",
            Format::Toml,
            "opencode",
        );
        assert!(!out.value.as_map().unwrap().contains_key("enabled"));
    }

    #[test]
    fn a_disabled_server_does_write_the_flag() {
        let doc = tree::parse(
            r#"{"mcp": {"x": {"type": "local", "command": ["a"], "enabled": false}}}"#,
            Format::Jsonc,
        )
        .unwrap();
        let read = crate::mcp::read_servers(
            &spec_for("opencode"),
            "opencode",
            doc.at_pointer("/mcp").unwrap().unwrap(),
        );
        let out = server(&spec_for("opencode"), "opencode", &read[0], &LiteralsOnly).unwrap();
        assert_eq!(out.value.as_map().unwrap().get("enabled"), Some(&Value::Bool(false)));
    }

    /// Fanning a credential out to another harness must say so. F-7.3 as
    /// originally written promised this could be avoided; it cannot.
    #[test]
    fn deploying_a_credential_elsewhere_warns_about_the_extra_copy() {
        let out = translate(
            "opencode",
            r#"{"mcp": {"x": {"type": "local", "command": ["srv"],
               "environment": {"API_KEY": "sekrit"}}}}"#,
            "/mcp",
            Format::Jsonc,
            "codex",
        );
        assert!(
            out.warnings.iter().any(|w| w.contains("cleartext")),
            "expected a cleartext warning, got {:?}",
            out.warnings
        );
    }

    /// Writing back where it came from is not a new copy, so no warning.
    #[test]
    fn rewriting_the_source_config_does_not_warn() {
        let out = translate(
            "opencode",
            r#"{"mcp": {"x": {"type": "local", "command": ["srv"],
               "environment": {"API_KEY": "sekrit"}}}}"#,
            "/mcp",
            Format::Jsonc,
            "opencode",
        );
        assert!(
            !out.warnings.iter().any(|w| w.contains("cleartext")),
            "should not warn when writing back to the source: {:?}",
            out.warnings
        );
    }

    /// An unresolvable secret must abort the emit, not write a placeholder.
    #[test]
    fn unresolvable_secret_aborts_the_emit() {
        let mut s = McpServer {
            name: "x".into(),
            transport: Transport::Stdio,
            command: "srv".into(),
            args: vec![],
            env: vec![("KEY".into(), Secret::Ref("missing".into()))],
            url: None,
            headers: vec![],
            enabled: true,
            extra: Map::default(),
            source_provider: "codex".into(),
        };
        assert!(server(&spec_for("codex"), "codex", &s, &LiteralsOnly).is_err());

        s.env = vec![("KEY".into(), Secret::Literal("v".into()))];
        assert!(server(&spec_for("codex"), "codex", &s, &LiteralsOnly).is_ok());
    }
}
