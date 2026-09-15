//! Round-trip fidelity (NF-1.x) — the invariant the project is built on.
//!
//! The corpus in `tests/corpus/` is mostly **real files copied off a working
//! machine** (secrets scrubbed), not synthetic fixtures. That matters: every
//! hazard here was found in the wild, not imagined.
//!
//! The headline property:
//!
//! > parse → render with no modification MUST reproduce the input bytes exactly.
//!
//! HarnessKit fails this for TOML (RESEARCH §5). It is the single clearest
//! demonstration that this tool is safe to point at a config you care about.

use inception_core::Editor;
use inception_core::spec::Format;
use inception_core::tree::{Map, Value};
use std::path::{Path, PathBuf};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

/// Read as bytes then convert, so line endings survive exactly. `read_to_string`
/// is fine on every platform here, but reading bytes makes the intent explicit.
fn read(name: &str) -> String {
    let path = corpus_dir().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    String::from_utf8(bytes).unwrap_or_else(|e| panic!("{} is not UTF-8: {e}", path.display()))
}

fn format_for(name: &str) -> Format {
    if name.ends_with(".toml") {
        Format::Toml
    } else {
        // Every .json is treated as JSONC — real ones contain comments.
        Format::Jsonc
    }
}

/// Files that carry content. Empty ones are covered separately: an empty file has
/// no bytes to preserve, and JSONC seeds it with `{}` on purpose.
fn corpus_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(corpus_dir())
        .expect("corpus directory")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".toml") || n.ends_with(".json") || n.ends_with(".jsonc"))
        .filter(|n| !n.starts_with("empty"))
        .collect();
    names.sort();
    names
}

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

// ---------------------------------------------------------------------------
// NF-1.1 — the headline property
// ---------------------------------------------------------------------------

#[test]
fn every_corpus_file_round_trips_byte_identically() {
    let files = corpus_files();
    assert!(files.len() >= 8, "corpus looks too small: {files:?}");

    for name in files {
        // "comments-only" has no root value to edit, but it must still survive
        // being parsed and rendered.
        let original = read(&name);
        let editor = Editor::parse(&original, format_for(&name))
            .unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));
        let rendered = editor.render();

        assert_eq!(
            rendered, original,
            "\n{name} did not round-trip byte-identically.\n\
             --- original ({} bytes) ---\n{original}\n\
             --- rendered ({} bytes) ---\n{rendered}\n",
            original.len(),
            rendered.len()
        );
    }
}

/// The specific case that motivated the whole project: Codex's real config, with
/// single-quoted literal strings holding Windows paths. A plain
/// parse→serialize re-emits these as escaped basic strings and reorders the
/// top-level keys alphabetically.
#[test]
fn real_codex_config_survives_untouched() {
    let original = read("codex-config.toml");
    assert!(
        original.contains(r"[projects.'D:\Downloads\blog-agents']"),
        "corpus file lost its literal-string table keys"
    );

    let rendered = Editor::parse(&original, Format::Toml).unwrap().render();
    assert_eq!(rendered, original);
    assert!(rendered.contains(r"[projects.'D:\Downloads\blog-agents']"));
    assert!(!rendered.contains(r#"[projects."D:\\Downloads"#));
}

/// NF-3.4: line endings are never normalised.
#[test]
fn crlf_file_keeps_its_line_endings() {
    let original = read("crlf.json");
    assert!(original.contains("\r\n"), "corpus file lost its CRLF endings");

    let rendered = Editor::parse(&original, Format::Jsonc).unwrap().render();
    assert_eq!(rendered, original);
    assert_eq!(
        rendered.matches("\r\n").count(),
        original.matches("\r\n").count()
    );
}

// ---------------------------------------------------------------------------
// NF-1.3 — surgical writes
// ---------------------------------------------------------------------------

/// Assert that `modified` contains every line of `original`, in order, unchanged
/// apart from a trailing comma.
///
/// **Why the comma exemption.** JSON requires a separator between properties, so
/// appending a sibling *necessarily* turns `"a": 1` into `"a": 1,` on the line
/// above. That is the syntax demanding it, not us reformatting. The generative
/// fuzz found this case; without the exemption the test was asserting something
/// no correct implementation could satisfy.
///
/// Everything else — reordering, requoting, comment loss, reindentation — still
/// fails here.
fn assert_only_additions(original: &str, modified: &str, context: &str) {
    fn matches(original_line: &str, modified_line: &str) -> bool {
        modified_line == original_line
            || modified_line.trim_end() == format!("{},", original_line.trim_end())
    }

    let mut wanted = original.lines();
    let mut next = wanted.next();

    for line in modified.lines() {
        if next.is_some_and(|want| matches(want, line)) {
            next = wanted.next();
        }
    }

    if let Some(missing) = next {
        panic!(
            "{context}: original line was modified or removed:\n  {missing:?}\n\
             --- original ---\n{original}\n--- modified ---\n{modified}"
        );
    }
}

/// Pins the comma behaviour explicitly, so the exemption above is documented by a
/// test rather than only by a comment.
#[test]
fn adding_a_json_property_only_adds_a_comma_to_the_previous_line() {
    let original = "{\n  \"a\": 1\n}\n";
    let mut editor = Editor::parse(original, Format::Jsonc).unwrap();
    editor.upsert("", "b", &Value::Int(2)).unwrap();
    let modified = editor.render();

    assert!(modified.contains("\"a\": 1,"), "expected a comma:\n{modified}");
    assert!(modified.contains("\"b\": 2"));
    // The value itself is untouched — only the separator was added.
    assert!(!modified.contains("\"a\": 1,,"));
}

#[test]
fn inserting_into_real_codex_config_changes_nothing_else() {
    let original = read("codex-config.toml");
    let mut editor = Editor::parse(&original, Format::Toml).unwrap();

    editor
        .upsert(
            "/mcp_servers",
            "obsidian",
            &map(&[
                ("command", s("uvx")),
                ("args", Value::Array(vec![s("mcp-obsidian")])),
                ("env", map(&[("OBSIDIAN_HOST", s("127.0.0.1"))])),
            ]),
        )
        .unwrap();
    let modified = editor.render();

    assert_only_additions(&original, &modified, "codex insert");
    assert!(modified.contains("[mcp_servers.obsidian]"));
    assert!(modified.contains("[mcp_servers.obsidian.env]"));

    // The pre-existing server is untouched, including its harness-specific field.
    assert!(modified.contains("[mcp_servers.node_repl]"));
    assert!(modified.contains("startup_timeout_sec = 120"));
}

#[test]
fn inserting_into_real_opencode_config_preserves_everything() {
    let original = read("opencode.jsonc");
    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();

    editor
        .upsert(
            "/mcp",
            "filesystem",
            &map(&[
                ("type", s("local")),
                ("command", Value::Array(vec![s("npx"), s("@mcp/fs")])),
            ]),
        )
        .unwrap();
    let modified = editor.render();

    assert!(modified.contains("\"filesystem\""));
    assert!(modified.contains("\"obsidian\""), "existing server lost");
    assert!(modified.contains("$schema"), "schema key lost");
}

/// NF-1.5: comments in a `.json` file survive an edit. `~/.copilot/config.json`
/// really does begin with two `//` lines.
#[test]
fn comments_in_a_json_file_survive_an_edit() {
    let original = read("copilot-config.json");
    assert!(original.contains("//"), "corpus file lost its comments");

    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();
    editor
        .upsert("/servers", "x", &map(&[("command", s("srv"))]))
        .unwrap();
    let modified = editor.render();

    assert!(modified.contains("// User settings belong in settings.json."));
    assert!(modified.contains("// This file is managed automatically."));
    assert!(modified.contains("\"firstLaunchAt\""));
    assert!(modified.contains("\"servers\""));
}

#[test]
fn jsonc_torture_file_keeps_every_comment_through_an_edit() {
    let original = read("jsonc-comments.json");
    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();
    editor
        .upsert("/servers", "three", &map(&[("command", s("c"))]))
        .unwrap();
    let modified = editor.render();

    for comment in [
        "// Leading line comment.",
        "/* block comment */",
        "// trailing comment",
        "// comment inside nested object",
    ] {
        assert!(modified.contains(comment), "lost {comment}:\n{modified}");
    }
    assert!(modified.contains("\"one\"") && modified.contains("\"two\""));
}

#[test]
fn every_toml_quoting_style_survives_an_edit() {
    let original = read("toml-quoting.toml");
    let mut editor = Editor::parse(&original, Format::Toml).unwrap();
    editor
        .upsert("/mcp_servers", "added", &map(&[("command", s("x"))]))
        .unwrap();
    let modified = editor.render();

    assert_only_additions(&original, &modified, "toml quoting");
    // Raw strings (r"..." / r#"..."#) so these read exactly as they appear in the
    // fixture — escaping them twice is how this test was wrong the first time.
    for fragment in [
        r"literal_path = 'C:\Users\User\escapes'",
        r#"basic_path = "C:\\Users\\User\\escaped""#,
        r"[projects.'D:\Downloads\blog-agents']",
        r#"[projects."quoted.dotted.key"]"#,
        "# Comment directly above a table.",
    ] {
        assert!(modified.contains(fragment), "lost: {fragment}\n{modified}");
    }
}

// ---------------------------------------------------------------------------
// The differentiator, as an executable claim
// ---------------------------------------------------------------------------

/// Head-to-head against the approach every competitor uses.
///
/// HarnessKit's `deployer.rs:451 upsert_mcp_server_toml()` does, in essence:
///
/// ```ignore
/// let mut doc: toml::Table = existing.parse()?;          // comments dropped here
/// doc.entry("mcp_servers")...insert(name, server_val);
/// atomic_write(path, &toml::to_string_pretty(&doc)?)?;   // whole file rewritten
/// ```
///
/// `toml::Table` is a `BTreeMap` with no representation for comments, so they
/// cannot survive, and iteration order is alphabetical rather than file order.
///
/// **Measured against `toml` 1.1, not asserted from memory.** The damage is:
///
/// | | naive | ours |
/// |---|---|---|
/// | comments | **all lost** | kept |
/// | key order | alphabetised | kept |
/// | string quoting | normalised, flipping both ways | kept |
/// | array layout | re-wrapped multi-line | kept |
/// | quoted table keys | *survive* | kept |
///
/// That last row corrects an earlier assumption: `[projects.'D:\...']` is **not**
/// rewritten by the naive path. The real damage is comments, ordering and value
/// quoting — which is more than enough, and is what this test pins down.
///
/// If this ever starts failing because the naive side stopped mangling things,
/// that is good news and the project's premise deserves re-examination.
#[test]
fn naive_serialize_mangles_the_file_and_our_editor_does_not() {
    // Uses the fixture that actually has comments; the real Codex config has none.
    let original = read("toml-quoting.toml");
    assert!(original.starts_with("# Top-of-file comment."), "fixture precondition");

    // --- the naive approach, as the competitors implement it --------------
    let mut naive: toml::Table = toml::from_str(&original).unwrap();
    naive.insert("added".to_string(), toml::Value::String("x".to_string()));
    let naive_out = toml::to_string_pretty(&naive).unwrap();

    let comments_before = original.lines().filter(|l| l.trim_start().starts_with('#')).count();
    let comments_after = naive_out.lines().filter(|l| l.trim_start().starts_with('#')).count();
    assert!(comments_before >= 2, "fixture precondition");
    assert_eq!(comments_after, 0, "naive path unexpectedly kept comments");

    // Wholesale reordering: the file no longer starts where it started.
    assert_ne!(original.lines().next(), naive_out.lines().next());

    // Value quoting is normalised — here a basic string became a literal one.
    assert!(original.contains(r#"basic_path = "C:\\Users\\User\\escaped""#));
    assert!(naive_out.contains(r"basic_path = 'C:\Users\User\escaped'"));

    // --- ours -------------------------------------------------------------
    let mut ours = Editor::parse(&original, Format::Toml).unwrap();
    ours.upsert("", "added", &s("x")).unwrap();
    let ours_out = ours.render();

    assert_only_additions(&original, &ours_out, "ours vs naive");
    assert_eq!(
        ours_out.lines().filter(|l| l.trim_start().starts_with('#')).count(),
        comments_before,
        "we lost a comment"
    );
    assert!(ours_out.contains(r#"basic_path = "C:\\Users\\User\\escaped""#));
    assert!(ours_out.contains(r"literal_path = 'C:\Users\User\escapes'"));
    assert!(ours_out.contains("added"));
}

// ---------------------------------------------------------------------------
// Algebraic properties
// ---------------------------------------------------------------------------

/// Writing the same value twice must not accumulate changes.
#[test]
fn upsert_is_idempotent() {
    for name in corpus_files() {
        let original = read(&name);
        let format = format_for(&name);
        let Ok(mut once) = Editor::parse(&original, format) else {
            continue;
        };
        if once.upsert("/servers", "probe", &map(&[("command", s("x"))])).is_err() {
            continue; // no root object (comments-only) — covered elsewhere
        }
        let after_one = once.render();

        let mut twice = Editor::parse(&after_one, format).unwrap();
        twice
            .upsert("/servers", "probe", &map(&[("command", s("x"))]))
            .unwrap();
        assert_eq!(twice.render(), after_one, "{name}: upsert is not idempotent");
    }
}

/// Insert then remove should land back where we started.
///
/// Whitespace may legitimately differ — removing a table can leave a blank line
/// the original did not have — so this asserts semantic identity, with the
/// stronger byte-identity check reserved for the no-op case above.
#[test]
fn insert_then_remove_restores_the_original_content() {
    let original = read("codex-config.toml");
    let mut editor = Editor::parse(&original, Format::Toml).unwrap();
    editor
        .upsert("/mcp_servers", "temp", &map(&[("command", s("x"))]))
        .unwrap();
    assert!(editor.render().contains("temp"));

    assert!(editor.remove("/mcp_servers", "temp").unwrap());
    let restored = editor.render();

    assert!(!restored.contains("[mcp_servers.temp]"));
    assert_only_additions(
        &original.replace("\n\n", "\n"),
        &restored.replace("\n\n", "\n"),
        "insert-then-remove",
    );
}

#[test]
fn removing_a_missing_key_is_a_no_op() {
    let original = read("codex-config.toml");
    let mut editor = Editor::parse(&original, Format::Toml).unwrap();
    assert!(!editor.remove("/mcp_servers", "not-there").unwrap());
    assert_eq!(editor.render(), original);
}

// ---------------------------------------------------------------------------
// T-2.12 — generative fuzz
// ---------------------------------------------------------------------------

/// Deterministic pseudo-random source: a linear congruential generator.
///
/// Seeded fixed so a failure is reproducible. A real randomised fuzzer that
/// fails once and never again is worse than useless in CI.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // Numerical Recipes LCG constants.
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() as usize) % n }
    }
}

/// Perturb each corpus file many ways and assert the identity property still
/// holds. Whitespace and comments are exactly the things a value-model round-trip
/// destroys, so that is what we inject.
///
/// Perturbations that produce invalid input (a comment landing inside a
/// multi-line string, say) are skipped — the property under test is
/// "valid input round-trips", not "anything round-trips".
#[test]
fn perturbed_corpus_files_still_round_trip() {
    let injections = [
        "# fuzz comment",
        "// fuzz comment",
        "",
        "   ",
        "\t",
        "/* fuzz block */",
    ];

    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut checked = 0usize;
    let mut skipped = 0usize;

    for name in corpus_files() {
        let original = read(&name);
        let format = format_for(&name);
        let lines: Vec<&str> = original.lines().collect();
        if lines.is_empty() {
            continue;
        }

        for _ in 0..60 {
            let at = rng.below(lines.len() + 1);
            let injection = injections[rng.below(injections.len())];

            let mut perturbed: Vec<&str> = lines.clone();
            perturbed.insert(at, injection);
            let text = perturbed.join("\n") + "\n";

            // Only valid documents are in scope.
            let Ok(editor) = Editor::parse(&text, format) else {
                skipped += 1;
                continue;
            };
            assert_eq!(
                editor.render(),
                text,
                "\n{name}: perturbed input did not round-trip \
                 (injected {injection:?} at line {at})\n--- input ---\n{text}"
            );
            checked += 1;
        }
    }

    // Guard against the test silently doing nothing.
    assert!(checked > 200, "fuzz only checked {checked} cases (skipped {skipped})");
}

/// Inserting into a perturbed document must still be surgical.
#[test]
fn inserting_into_perturbed_documents_stays_surgical() {
    let mut rng = Rng(0xfeed_face_dead_beef);

    for name in corpus_files() {
        let original = read(&name);
        let format = format_for(&name);
        let lines: Vec<&str> = original.lines().collect();
        if lines.is_empty() {
            continue;
        }

        for _ in 0..20 {
            let at = rng.below(lines.len() + 1);
            let mut perturbed: Vec<&str> = lines.clone();
            perturbed.insert(at, if format == Format::Toml { "# c" } else { "// c" });
            let text = perturbed.join("\n") + "\n";

            let Ok(mut editor) = Editor::parse(&text, format) else {
                continue;
            };
            if editor
                .upsert("/fuzz_target", "entry", &map(&[("command", s("x"))]))
                .is_err()
            {
                continue; // no root object to write into
            }
            assert_only_additions(&text, &editor.render(), &format!("{name} perturbed"));
        }
    }
}

// ---------------------------------------------------------------------------
// RESEARCH §4 hazards (T-1.23)
// ---------------------------------------------------------------------------

/// Three distinct "nothing here" states, all real on the dev machine.
#[test]
fn empty_files_are_handled_not_crashed() {
    // TOML: an empty document is already a valid table.
    let toml = Editor::parse(&read("empty.toml"), Format::Toml).unwrap();
    assert_eq!(toml.render(), "");

    // JSONC: seeded with `{}` so there is somewhere to write.
    let mut jsonc = Editor::parse(&read("empty.json"), Format::Jsonc).unwrap();
    jsonc.upsert("/servers", "a", &map(&[("command", s("x"))])).unwrap();
    assert!(jsonc.render().contains("\"a\""));
}

/// A file of only comments parses and renders unchanged, but has no root object
/// to write into. Failing loudly beats silently discarding the comments.
#[test]
fn comments_only_file_round_trips_but_refuses_writes() {
    let original = read("comments-only.json");
    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();
    assert_eq!(editor.render(), original);

    let err = editor
        .upsert("/servers", "a", &s("x"))
        .expect_err("writing into a document with no root object should fail");
    assert!(err.to_string().contains("not an object"), "got: {err}");
}

/// Claude Code's `"mcpServers": null` must not be mistaken for an empty object
/// and quietly overwritten in a way that loses the distinction.
#[test]
fn null_valued_key_can_be_replaced_without_touching_siblings() {
    let original = read("claude-trimmed.json");
    assert!(original.contains("\"mcpServers\": null"));

    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();
    editor
        .upsert("", "mcpServers", &map(&[("x", map(&[("command", s("a"))]))]))
        .unwrap();
    let modified = editor.render();

    assert!(!modified.contains("\"mcpServers\": null"));
    assert!(modified.contains("\"hasCompletedOnboarding\""));
    assert!(modified.contains("\"C:/Users/User\""));
}

/// The dynamic-pointer case, written through to a real file shape.
#[test]
fn writing_through_a_project_namespaced_pointer_hits_one_key() {
    let original = read("claude-trimmed.json");
    let mut editor = Editor::parse(&original, Format::Jsonc).unwrap();

    let escaped = inception_core::pointer::escape_token("D:/Projects/project-inception");
    editor
        .upsert(
            &format!("/projects/{escaped}/mcpServers"),
            "local-server",
            &map(&[("command", s("node"))]),
        )
        .unwrap();
    let modified = editor.render();

    assert!(modified.contains("\"local-server\""));
    // The other project is untouched and no stray nesting was created.
    assert_eq!(modified.matches("\"D:/Projects/project-inception\"").count(), 1);
    assert!(modified.contains("\"notion\""), "other project's server lost");
}
