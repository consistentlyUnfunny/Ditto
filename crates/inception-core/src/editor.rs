//! Format-preserving config editing (NF-1.x) — **the moat**.
//!
//! Every competitor reads a config into a plain value model and serialises it
//! back out. That destroys comments, key order and quoting style. HarnessKit's
//! `upsert_mcp_server_toml` does exactly this to `~/.codex/config.toml`
//! (RESEARCH §5).
//!
//! We never do that. Editing goes through a **concrete syntax tree** — a parse
//! tree that still remembers whitespace, comments and how each string was
//! quoted. You mutate one node; every other byte is passed through untouched.
//!
//! In terms a JS reader will recognise: `JSON.parse` → `JSON.stringify` is the
//! wrong model, Prettier's parser is the right one.
//!
//!   - TOML  → `toml_edit` (what `cargo add` uses on your `Cargo.toml`)
//!   - JSONC → `jsonc-parser`'s CST (what VS Code uses on your settings.json)
//!
//! The invariant, enforced by `tests/round_trip.rs`: **parse then render with no
//! modification must reproduce the input bytes exactly.**

use crate::error::{Error, Result};
use crate::pointer;
use crate::spec::Format;
use crate::tree::Value;
use std::path::PathBuf;

pub struct Editor {
    kind: Kind,
}

enum Kind {
    Toml(Box<toml_edit::DocumentMut>),
    Jsonc(Box<jsonc_parser::cst::CstRootNode>),
}

/// Hand-written because neither underlying tree implements `Debug`, and printing
/// a whole config into a panic message would be noise anyway.
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            Kind::Toml(_) => "Toml",
            Kind::Jsonc(_) => "Jsonc",
        };
        write!(f, "Editor({kind}, {} bytes)", self.render().len())
    }
}

impl Editor {
    /// Parse text for editing.
    ///
    /// An empty JSONC file is seeded with `{}` so there is a root object to write
    /// into; TOML needs no seeding because an empty document is already a table.
    pub fn parse(text: &str, format: Format) -> Result<Editor> {
        let kind = match format {
            Format::Toml => {
                let doc = text
                    .parse::<toml_edit::DocumentMut>()
                    .map_err(|e| Error::ConfigParse {
                        path: PathBuf::new(),
                        format: "TOML",
                        reason: e.to_string(),
                    })?;
                Kind::Toml(Box::new(doc))
            }
            Format::Jsonc => {
                let seed = if text.trim().is_empty() { "{}" } else { text };
                let root = jsonc_parser::cst::CstRootNode::parse(seed, &Default::default())
                    .map_err(|e| Error::ConfigParse {
                        path: PathBuf::new(),
                        format: "JSONC",
                        reason: e.to_string(),
                    })?;
                Kind::Jsonc(Box::new(root))
            }
            // No built-in provider declares a YAML config file, and skills are
            // deployed by copying the directory rather than by rewriting
            // frontmatter — so nothing needs a YAML *writer* yet. Refusing is
            // honest; a half-working one that silently drops comments is not.
            Format::Yaml => {
                return Err(Error::ConfigParse {
                    path: PathBuf::new(),
                    format: "YAML",
                    reason: "format-preserving YAML editing is not implemented; \
                             no provider needs it (see editor.rs)"
                        .to_string(),
                });
            }
        };
        Ok(Editor { kind })
    }

    /// Render the document back to text.
    pub fn render(&self) -> String {
        match &self.kind {
            Kind::Toml(doc) => doc.to_string(),
            Kind::Jsonc(root) => root.to_string(),
        }
    }

    /// Insert or replace `key` inside the object addressed by `container`
    /// (a JSON Pointer), creating intermediate containers as needed.
    ///
    /// Only the affected entry's lines change (NF-1.3).
    pub fn upsert(&mut self, container: &str, key: &str, value: &Value) -> Result<()> {
        let tokens = pointer::parse(container)?;
        match &mut self.kind {
            Kind::Toml(doc) => upsert_toml(doc, &tokens, key, value),
            Kind::Jsonc(root) => upsert_jsonc(root, &tokens, key, value),
        }
    }

    /// Like [`upsert`](Self::upsert), but **merges** into an existing entry
    /// instead of replacing it: keys present in the file that this call does not
    /// mention are left exactly as they are.
    ///
    /// This is PROVIDER-SPEC §6 rule 3, and it is the difference between a safe
    /// update and silent data loss. Updating a Codex server with
    /// `{command, args}` must not delete the `startup_timeout_sec = 120` the user
    /// set by hand; a plain replace would.
    ///
    /// Merging is shallow by design. A nested map (`env`) is replaced wholesale,
    /// because "the user's env plus mine" has no correct answer — deleting a var
    /// would be impossible, and silently unioning them would resurrect variables
    /// the caller meant to drop.
    pub fn merge(&mut self, container: &str, key: &str, value: &Value) -> Result<()> {
        let Value::Map(fields) = value else {
            // A scalar has no fields to merge; replacing is the only meaning.
            return self.upsert(container, key, value);
        };

        let nested = match container {
            "" => format!("/{}", pointer::escape_token(key)),
            _ => format!("{container}/{}", pointer::escape_token(key)),
        };

        // Nothing there yet, or something that is not an object: plain write.
        if !self.container_exists(&nested)? {
            return self.upsert(container, key, value);
        }

        for (field, field_value) in fields.iter() {
            self.upsert(&nested, field, field_value)?;
        }
        Ok(())
    }

    /// Whether the pointer addresses an existing map/table we can merge into.
    fn container_exists(&self, pointer_str: &str) -> Result<bool> {
        let tokens = pointer::parse(pointer_str)?;
        Ok(match &self.kind {
            Kind::Toml(doc) => {
                let mut current = doc.as_table().get(tokens.first().map_or("", |s| s.as_str()));
                if tokens.is_empty() {
                    return Ok(true);
                }
                for token in &tokens[1..] {
                    current = current.and_then(|item| item.as_table()).and_then(|t| t.get(token));
                }
                current.is_some_and(|item| item.as_table().is_some())
            }
            Kind::Jsonc(root) => {
                let Some(mut current) = root.object_value() else {
                    return Ok(false);
                };
                for token in &tokens {
                    match current.object_value(token) {
                        Some(next) => current = next,
                        None => return Ok(false),
                    }
                }
                true
            }
        })
    }

    /// Remove `key` from the object addressed by `container`.
    /// Returns whether anything was removed.
    pub fn remove(&mut self, container: &str, key: &str) -> Result<bool> {
        let tokens = pointer::parse(container)?;
        match &mut self.kind {
            Kind::Toml(doc) => Ok(remove_toml(doc, &tokens, key)),
            Kind::Jsonc(root) => Ok(remove_jsonc(root, &tokens, key)),
        }
    }
}

// ---------------------------------------------------------------------------
// TOML
// ---------------------------------------------------------------------------

fn upsert_toml(
    doc: &mut toml_edit::DocumentMut,
    tokens: &[String],
    key: &str,
    value: &Value,
) -> Result<()> {
    let table = toml_container(doc.as_table_mut(), tokens)?;
    table.insert(key, to_toml_item(value));
    Ok(())
}

fn remove_toml(doc: &mut toml_edit::DocumentMut, tokens: &[String], key: &str) -> bool {
    match toml_container(doc.as_table_mut(), tokens) {
        Ok(table) => table.remove(key).is_some(),
        Err(_) => false,
    }
}

/// Walk to (or create) the table a pointer addresses.
///
/// Intermediate tables we create are marked *implicit*, so `[mcp_servers]` is
/// not emitted as a bare header when only `[mcp_servers.obsidian]` has content —
/// that is the shape Codex's own file already uses.
fn toml_container<'a>(
    root: &'a mut toml_edit::Table,
    tokens: &[String],
) -> Result<&'a mut toml_edit::Table> {
    let mut current = root;
    for token in tokens {
        if !current.contains_key(token) {
            let mut fresh = toml_edit::Table::new();
            fresh.set_implicit(true);
            current.insert(token, toml_edit::Item::Table(fresh));
        }
        current = current
            .get_mut(token)
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or_else(|| Error::BadPointer {
                pointer: tokens.join("/"),
                reason: format!("'{token}' exists but is not a table"),
            })?;
    }
    Ok(current)
}

/// Maps become sub-tables (`[mcp_servers.x.env]`) rather than inline tables,
/// matching how the harnesses write these files themselves.
fn to_toml_item(value: &Value) -> toml_edit::Item {
    match value {
        Value::Map(map) => {
            let mut table = toml_edit::Table::new();
            for (k, v) in map.iter() {
                table.insert(k, to_toml_item(v));
            }
            toml_edit::Item::Table(table)
        }
        other => toml_edit::Item::Value(to_toml_value(other)),
    }
}

fn to_toml_value(value: &Value) -> toml_edit::Value {
    use toml_edit::Value as V;
    match value {
        Value::Null => V::from(""), // TOML has no null; empty string is the least-bad
        Value::Bool(b) => V::from(*b),
        Value::Int(i) => V::from(*i),
        Value::Float(f) => V::from(*f),
        Value::String(s) => V::from(s.as_str()),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(to_toml_value(item));
            }
            V::Array(array)
        }
        Value::Map(map) => {
            // Reached only for a map nested inside an array, where TOML requires
            // an inline table.
            let mut inline = toml_edit::InlineTable::new();
            for (k, v) in map.iter() {
                inline.insert(k, to_toml_value(v));
            }
            V::InlineTable(inline)
        }
    }
}

// ---------------------------------------------------------------------------
// JSONC
// ---------------------------------------------------------------------------

fn upsert_jsonc(
    root: &mut jsonc_parser::cst::CstRootNode,
    tokens: &[String],
    key: &str,
    value: &Value,
) -> Result<()> {
    let object = jsonc_container(root, tokens)?;
    match object.get(key) {
        // Replace just this property's value; siblings and their comments stay.
        Some(existing) => existing.set_value(to_cst(value)),
        None => {
            object.append(key, to_cst(value));
        }
    }
    Ok(())
}

fn remove_jsonc(
    root: &mut jsonc_parser::cst::CstRootNode,
    tokens: &[String],
    key: &str,
) -> bool {
    match jsonc_container(root, tokens) {
        Ok(object) => match object.get(key) {
            Some(prop) => {
                prop.remove();
                true
            }
            None => false,
        },
        Err(_) => false,
    }
}

fn jsonc_container(
    root: &jsonc_parser::cst::CstRootNode,
    tokens: &[String],
) -> Result<jsonc_parser::cst::CstObject> {
    let mut current = root.object_value().ok_or_else(|| Error::ConfigParse {
        path: PathBuf::new(),
        format: "JSONC",
        reason: "document root is not an object".to_string(),
    })?;
    for token in tokens {
        current = current.object_value_or_set(token);
    }
    Ok(current)
}

fn to_cst(value: &Value) -> jsonc_parser::cst::CstInputValue {
    use jsonc_parser::cst::CstInputValue as C;
    match value {
        Value::Null => C::Null,
        Value::Bool(b) => C::Bool(*b),
        Value::Int(i) => C::Number(i.to_string()),
        Value::Float(f) => C::Number(f.to_string()),
        Value::String(s) => C::String(s.clone()),
        Value::Array(items) => C::Array(items.iter().map(to_cst).collect()),
        Value::Map(map) => C::Object(
            map.iter()
                .map(|(k, v)| (k.to_string(), to_cst(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Map;

    fn map(pairs: &[(&str, Value)]) -> Value {
        let mut m = Map::default();
        for (k, v) in pairs {
            m.insert(*k, v.clone());
        }
        Value::Map(m)
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    /// NF-1.3 in its purest form: a comment above an untouched table, a literal
    /// string, and key order must all be exactly as they were.
    #[test]
    fn toml_insert_touches_only_the_new_entry() {
        let original = "\
# Keep me.
model = 'gpt-5'

[projects.'D:\\Downloads\\blog']
trust_level = \"trusted\"
";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        editor
            .upsert(
                "/mcp_servers",
                "obsidian",
                &map(&[
                    ("command", s("uvx")),
                    ("args", Value::Array(vec![s("mcp-obsidian")])),
                ]),
            )
            .unwrap();
        let result = editor.render();

        // Everything that was there is still there, byte for byte.
        assert!(result.starts_with("# Keep me.\nmodel = 'gpt-5'\n"));
        assert!(
            result.contains("[projects.'D:\\Downloads\\blog']"),
            "literal-string table key was rewritten:\n{result}"
        );
        assert!(result.contains("[mcp_servers.obsidian]"));
        assert!(result.contains("command = \"uvx\""));
    }

    /// The specific regression this project exists to prevent.
    #[test]
    fn toml_literal_strings_are_not_converted_to_escaped_basic_strings() {
        let original = "path = 'C:\\Users\\User\\node.exe'\n";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        editor.upsert("", "added", &s("x")).unwrap();
        let result = editor.render();
        assert!(
            result.contains(r"path = 'C:\Users\User\node.exe'"),
            "literal string was rewritten:\n{result}"
        );
        assert!(!result.contains(r#""C:\\Users"#));
    }

    #[test]
    fn toml_nested_map_becomes_a_subtable() {
        let mut editor = Editor::parse("", Format::Toml).unwrap();
        editor
            .upsert(
                "/mcp_servers",
                "x",
                &map(&[
                    ("command", s("srv")),
                    ("env", map(&[("KEY", s("value"))])),
                ]),
            )
            .unwrap();
        let result = editor.render();
        assert!(result.contains("[mcp_servers.x]"));
        assert!(result.contains("[mcp_servers.x.env]"), "got:\n{result}");
        // `[mcp_servers]` itself stays implicit — no bare header.
        assert!(!result.contains("\n[mcp_servers]\n"), "got:\n{result}");
    }

    #[test]
    fn toml_upsert_replaces_an_existing_entry() {
        let original = "[mcp_servers.x]\ncommand = \"old\"\n";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        editor
            .upsert("/mcp_servers", "x", &map(&[("command", s("new"))]))
            .unwrap();
        let result = editor.render();
        assert!(result.contains("command = \"new\""));
        assert!(!result.contains("old"));
    }

    #[test]
    fn toml_remove_deletes_only_that_entry() {
        let original = "[mcp_servers.keep]\ncommand = \"a\"\n\n[mcp_servers.drop]\ncommand = \"b\"\n";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        assert!(editor.remove("/mcp_servers", "drop").unwrap());
        let result = editor.render();
        assert!(result.contains("[mcp_servers.keep]"));
        assert!(!result.contains("drop"));
    }

    /// NF-1.5: comments in a JSON file survive a write.
    #[test]
    fn jsonc_insert_preserves_comments() {
        let original = "\
// Leading comment.
{
  /* block */
  \"alpha\": 1, // trailing
  \"servers\": {
    \"one\": { \"command\": \"a\" }
  }
}
";
        let mut editor = Editor::parse(original, Format::Jsonc).unwrap();
        editor
            .upsert("/servers", "two", &map(&[("command", s("b"))]))
            .unwrap();
        let result = editor.render();

        assert!(result.contains("// Leading comment."));
        assert!(result.contains("/* block */"));
        assert!(result.contains("// trailing"));
        assert!(result.contains("\"two\""));
        assert!(result.contains("\"one\""), "existing entry lost:\n{result}");
    }

    #[test]
    fn jsonc_creates_missing_containers() {
        let mut editor = Editor::parse("{}", Format::Jsonc).unwrap();
        editor
            .upsert("/mcp", "obsidian", &map(&[("command", s("uvx"))]))
            .unwrap();
        let result = editor.render();
        assert!(result.contains("\"mcp\""));
        assert!(result.contains("\"obsidian\""));
    }

    /// The Windows-path-as-object-key case, end to end through the editor.
    #[test]
    fn jsonc_writes_through_an_escaped_windows_path_pointer() {
        let original = "{\n  \"projects\": {\n    \"C:/Users/User\": {}\n  }\n}\n";
        let mut editor = Editor::parse(original, Format::Jsonc).unwrap();
        let escaped = pointer::escape_token("C:/Users/User");
        editor
            .upsert(
                &format!("/projects/{escaped}/mcpServers"),
                "notion",
                &map(&[("type", s("http")), ("url", s("https://e.com"))]),
            )
            .unwrap();
        let result = editor.render();
        assert!(result.contains("\"mcpServers\""));
        assert!(result.contains("\"notion\""));
        // One project key, not a nested C:/Users/User tree.
        assert_eq!(result.matches("\"C:/Users/User\"").count(), 1, "got:\n{result}");
    }

    #[test]
    fn jsonc_remove_keeps_siblings_and_comments() {
        let original = "{\n  // keep this\n  \"a\": 1,\n  \"b\": 2\n}\n";
        let mut editor = Editor::parse(original, Format::Jsonc).unwrap();
        assert!(editor.remove("", "b").unwrap());
        let result = editor.render();
        assert!(result.contains("// keep this"));
        assert!(result.contains("\"a\""));
        assert!(!result.contains("\"b\""));
    }

    #[test]
    fn empty_jsonc_is_seeded_with_an_object() {
        let mut editor = Editor::parse("", Format::Jsonc).unwrap();
        editor.upsert("", "a", &s("b")).unwrap();
        assert!(editor.render().contains("\"a\""));
    }

    // --- merge: PROVIDER-SPEC §6 rule 3 -----------------------------------

    /// The data-loss case. Updating a Codex server must not delete the
    /// `startup_timeout_sec` the user set by hand.
    #[test]
    fn toml_merge_keeps_fields_the_caller_did_not_mention() {
        let original = "\
[mcp_servers.node_repl]
command = 'old.exe'
startup_timeout_sec = 120
tool_timeout_sec = 300
";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        editor
            .merge("/mcp_servers", "node_repl", &map(&[("command", s("new.exe"))]))
            .unwrap();
        let result = editor.render();

        assert!(result.contains("command = \"new.exe\""));
        assert!(result.contains("startup_timeout_sec = 120"), "lost a field:\n{result}");
        assert!(result.contains("tool_timeout_sec = 300"), "lost a field:\n{result}");
    }

    /// The contrast: plain `upsert` replaces, which is why `merge` exists.
    #[test]
    fn toml_upsert_replaces_whereas_merge_preserves() {
        let original = "[mcp_servers.x]\ncommand = 'a'\nstartup_timeout_sec = 120\n";

        let mut replacing = Editor::parse(original, Format::Toml).unwrap();
        replacing
            .upsert("/mcp_servers", "x", &map(&[("command", s("b"))]))
            .unwrap();
        assert!(!replacing.render().contains("startup_timeout_sec"));

        let mut merging = Editor::parse(original, Format::Toml).unwrap();
        merging
            .merge("/mcp_servers", "x", &map(&[("command", s("b"))]))
            .unwrap();
        assert!(merging.render().contains("startup_timeout_sec = 120"));
    }

    #[test]
    fn jsonc_merge_keeps_unmentioned_keys_and_comments() {
        let original = "\
{
  \"mcp\": {
    \"obsidian\": {
      // keep this comment
      \"type\": \"local\",
      \"enabled\": true
    }
  }
}
";
        let mut editor = Editor::parse(original, Format::Jsonc).unwrap();
        editor
            .merge("/mcp", "obsidian", &map(&[("type", s("remote"))]))
            .unwrap();
        let result = editor.render();

        assert!(result.contains("\"remote\""));
        assert!(result.contains("\"enabled\": true"), "lost a key:\n{result}");
        assert!(result.contains("// keep this comment"));
    }

    #[test]
    fn merge_on_a_missing_entry_creates_it() {
        let mut editor = Editor::parse("", Format::Toml).unwrap();
        editor
            .merge("/mcp_servers", "fresh", &map(&[("command", s("x"))]))
            .unwrap();
        assert!(editor.render().contains("[mcp_servers.fresh]"));
    }

    /// Nested maps replace wholesale — see the doc comment on `merge` for why
    /// a union would be wrong.
    #[test]
    fn merge_replaces_nested_maps_rather_than_unioning_them() {
        let original = "[s.x]\ncommand = 'a'\n\n[s.x.env]\nOLD = 'gone'\n";
        let mut editor = Editor::parse(original, Format::Toml).unwrap();
        editor
            .merge("/s", "x", &map(&[("env", map(&[("NEW", s("here"))]))]))
            .unwrap();
        let result = editor.render();

        assert!(result.contains("NEW"));
        assert!(!result.contains("OLD"), "env should be replaced, not unioned:\n{result}");
        assert!(result.contains("command = 'a'"), "sibling field lost:\n{result}");
    }

    /// Refusing is better than silently mangling (see the module docs).
    #[test]
    fn yaml_editing_is_refused_explicitly() {
        let err = Editor::parse("a: b\n", Format::Yaml).unwrap_err();
        assert!(err.to_string().contains("not implemented"));
    }

    /// NF-1.6: a corrupt file is never opened for writing.
    #[test]
    fn invalid_input_is_rejected() {
        assert!(Editor::parse("not [[[ toml", Format::Toml).is_err());
        assert!(Editor::parse("{ not json", Format::Jsonc).is_err());
    }
}
