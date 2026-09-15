//! Deploy planning and execution against a synthetic five-harness machine.
//!
//! Builds a throwaway "home" directory containing all five harnesses, so the
//! planner can be exercised end to end without touching the real one. The
//! headline assertions are the MVP acceptance criteria from
//! `docs/REQUIREMENTS.md`.

use inception_core::plan::{Plan, Step};
use inception_core::secrets::LiteralsOnly;
use inception_core::spec::Scope;
use inception_core::{PathContext, Registry};
use std::path::{Path, PathBuf};

/// A temporary home directory that removes itself.
struct FakeHome(PathBuf);

impl FakeHome {
    fn new(tag: &str) -> FakeHome {
        let dir = std::env::temp_dir().join(format!(
            "inception-deploy-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Enough of each harness to be detected as installed.
        for sub in [
            ".claude",
            ".claude/skills",
            ".codex",
            ".config/opencode",
            ".config/opencode/skills",
            ".gemini",
            ".copilot",
            ".agents/skills",
        ] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join(".claude.json"), "{}\n").unwrap();
        std::fs::write(dir.join(".codex/config.toml"), "model = \"gpt-5\"\n").unwrap();
        std::fs::write(
            dir.join(".config/opencode/opencode.jsonc"),
            "{\n  // keep me\n  \"mcp\": {}\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join(".gemini/settings.json"), "{}\n").unwrap();
        std::fs::write(dir.join(".copilot/mcp-config.json"), "{}\n").unwrap();

        FakeHome(dir)
    }

    fn ctx(&self) -> PathContext {
        PathContext {
            user_home: self.0.clone(),
            config: self.0.join(".config"),
            project: None,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// A source skill directory with a bundled resource file.
    fn make_skill(&self, name: &str) -> PathBuf {
        let dir = self.0.join("source").join(name);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: A test skill.\n---\n\nBody.\n"),
        )
        .unwrap();
        std::fs::write(dir.join("scripts/helper.sh"), "echo hi\n").unwrap();
        dir
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn all_targets() -> Vec<String> {
    ["claude-code", "codex", "copilot-cli", "gemini-cli", "opencode"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn skill_steps(plan: &Plan) -> Vec<&inception_core::plan::SkillWrite> {
    plan.steps
        .iter()
        .filter_map(|s| match s {
            Step::Skill(w) => Some(w),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// MVP #7 / F-5.2 — the minimum write set
// ---------------------------------------------------------------------------

/// **The headline claim.** Deploying one skill to all five harnesses must produce
/// exactly two writes: one to the shared `~/.agents/skills`, one to Claude Code's
/// own directory — because Claude Code is the only harness that refuses the
/// shared path.
#[test]
fn deploying_a_skill_everywhere_takes_exactly_two_writes() {
    let home = FakeHome::new("minwrite");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    assert_eq!(providers.iter().filter(|p| p.installed).count(), 5);

    let source = home.make_skill("karpathy-coder");
    let plan = inception_core::plan::skill_deploy(&providers, &all_targets(), &source, Scope::User);

    assert!(plan.refusals.is_empty(), "unexpected refusals: {:?}", plan.refusals);

    let writes = skill_steps(&plan);
    assert_eq!(
        writes.len(),
        2,
        "expected 2 writes, got {}: {:#?}",
        writes.len(),
        writes.iter().map(|w| (&w.dest, &w.providers)).collect::<Vec<_>>()
    );

    // One shared write covering the four harnesses that read ~/.agents/skills.
    let shared = writes.iter().find(|w| w.providers.len() > 1).expect("a shared write");
    assert_eq!(shared.providers.len(), 4);
    assert!(shared.dest.starts_with(home.path(".agents/skills")));
    assert!(!shared.providers.contains(&"claude-code".to_string()));
    assert!(shared.reason.contains("shared"));

    // And Claude Code on its own.
    let solo = writes.iter().find(|w| w.providers.len() == 1).expect("a solo write");
    assert_eq!(solo.providers, vec!["claude-code"]);
    assert!(solo.dest.starts_with(home.path(".claude/skills")));

    // Every target is accounted for — a smaller write set must not mean
    // silently dropping someone.
    assert_eq!(plan.providers_served().len(), 5);
}

/// A single-target deploy uses that harness's own directory, with no collapsing.
#[test]
fn deploying_to_one_harness_uses_its_own_directory() {
    let home = FakeHome::new("single");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let source = home.make_skill("solo");

    let plan = inception_core::plan::skill_deploy(
        &providers,
        &["opencode".to_string()],
        &source,
        Scope::User,
    );
    let writes = skill_steps(&plan);
    assert_eq!(writes.len(), 1);
    assert!(writes[0].dest.starts_with(home.path(".config/opencode/skills")));
}

/// Codex's only non-deprecated user directory *is* the shared one, so a
/// Codex-only deploy is visible to other harnesses. The plan must say so rather
/// than imply the write was private.
#[test]
fn codex_only_deploy_admits_it_lands_in_a_shared_directory() {
    let home = FakeHome::new("codexonly");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let source = home.make_skill("x");

    let plan =
        inception_core::plan::skill_deploy(&providers, &["codex".to_string()], &source, Scope::User);
    let writes = skill_steps(&plan);
    assert_eq!(writes.len(), 1);
    assert!(writes[0].dest.starts_with(home.path(".agents/skills")));
    assert!(
        writes[0].reason.contains("other harnesses also read"),
        "reason should warn about visibility, got: {}",
        writes[0].reason
    );
}

/// F-5.1 / F-5.6: a real copy, including bundled resource files, no symlinks.
#[test]
fn applying_a_skill_plan_copies_the_whole_directory() {
    let home = FakeHome::new("copy");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let source = home.make_skill("bundled");

    let plan = inception_core::plan::skill_deploy(&providers, &all_targets(), &source, Scope::User);
    let outcomes = inception_core::plan::apply(&plan);

    assert!(outcomes.iter().all(|o| o.result.is_ok()), "{outcomes:#?}");

    for rel in [".agents/skills/bundled", ".claude/skills/bundled"] {
        let dir = home.path(rel);
        assert!(dir.join("SKILL.md").is_file(), "missing SKILL.md in {rel}");
        assert!(dir.join("scripts/helper.sh").is_file(), "missing bundled file in {rel}");
        // Copies, not links.
        assert!(!dir.symlink_metadata().unwrap().file_type().is_symlink());
    }
}

// ---------------------------------------------------------------------------
// MVP #6 — MCP deploy, and the diff that proves it was surgical
// ---------------------------------------------------------------------------

fn obsidian_server() -> inception_core::McpServer {
    use inception_core::mcp::Secret;
    inception_core::McpServer {
        name: "obsidian".into(),
        transport: inception_core::spec::Transport::Stdio,
        command: "uvx".into(),
        args: vec!["mcp-obsidian".into()],
        env: vec![
            ("OBSIDIAN_HOST".into(), Secret::Literal("127.0.0.1".into())),
            ("OBSIDIAN_PORT".into(), Secret::Literal("27124".into())),
        ],
        url: None,
        headers: vec![],
        enabled: true,
        extra: Default::default(),
        source_provider: "opencode".into(),
    }
}

/// The MVP acceptance case: deploy one server to Codex and Claude Code, and the
/// Codex diff must contain *only* the added block.
#[test]
fn deploying_an_mcp_server_writes_only_the_new_entry() {
    let home = FakeHome::new("mcp");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let targets = vec!["codex".to_string(), "claude-code".to_string()];
    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &targets,
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    assert_eq!(plan.steps.len(), 2);

    // The preview is generated from the real edit, not a description of it.
    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(preview.contains("+[mcp_servers.obsidian]"), "preview:\n{preview}");

    let codex_before = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    let outcomes = inception_core::plan::apply(&plan);
    assert!(outcomes.iter().all(|o| o.result.is_ok()), "{outcomes:#?}");

    // --- Codex: TOML, argv split, env sub-table ---------------------------
    let codex = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    assert!(codex.starts_with(&codex_before), "existing content was rewritten:\n{codex}");
    assert!(codex.contains("[mcp_servers.obsidian]"));
    assert!(codex.contains("command = \"uvx\""));
    assert!(codex.contains("[mcp_servers.obsidian.env]"));
    assert!(!codex.contains("\"type\""), "Codex has no transport key");

    // --- Claude Code: JSON, split command/args, no type for stdio ---------
    let claude = std::fs::read_to_string(home.path(".claude.json")).unwrap();
    assert!(claude.contains("\"obsidian\""));
    assert!(claude.contains("\"mcpServers\""));
    assert!(claude.contains("\"uvx\""));
    assert!(!claude.contains("\"type\""));
}

/// NF-2.2: every write is reversible.
#[test]
fn deploying_leaves_a_recoverable_backup() {
    let home = FakeHome::new("backup");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let before = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    let outcomes = inception_core::plan::apply(&plan);

    let message = outcomes[0].result.as_ref().unwrap();
    assert!(message.contains("backup"), "got: {message}");

    // The backup really holds the previous contents.
    let backup_path = message
        .split_once("backup: ")
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .expect("backup path in message");
    assert_eq!(std::fs::read_to_string(backup_path).unwrap(), before);
    let _ = std::fs::remove_file(backup_path);
}

/// OpenCode's comments must survive a deploy into its config.
#[test]
fn deploying_into_opencode_preserves_its_comments() {
    let home = FakeHome::new("opencomment");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["opencode".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    inception_core::plan::apply(&plan);

    let text = std::fs::read_to_string(home.path(".config/opencode/opencode.jsonc")).unwrap();
    assert!(text.contains("// keep me"), "comment lost:\n{text}");
    assert!(text.contains("\"obsidian\""));
    // OpenCode's own shape: merged argv array and `environment`.
    assert!(text.contains("\"uvx\"") && text.contains("\"mcp-obsidian\""));
    assert!(text.contains("environment"));
}

// ---------------------------------------------------------------------------
// Refusals — never a silent skip (F-5.4, NF-2.4)
// ---------------------------------------------------------------------------

#[test]
fn unsupported_transport_is_refused_with_an_explanation() {
    let home = FakeHome::new("refuse");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let mut server = obsidian_server();
    server.transport = inception_core::spec::Transport::Ws;
    server.command = String::new();
    server.url = Some("wss://example.com".into());

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string(), "claude-code".to_string()],
        &server,
        Scope::User,
        &LiteralsOnly,
    );

    // Claude Code supports ws; Codex does not.
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.refusals.len(), 1);
    assert_eq!(plan.refusals[0].provider, "codex");
    assert!(plan.refusals[0].reason.contains("ws"));
    assert!(plan.refusals[0].reason.contains("stdio"), "should list what IS supported");
}

#[test]
fn an_unknown_target_is_refused_not_ignored() {
    let home = FakeHome::new("unknown");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["not-a-harness".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    assert!(plan.steps.is_empty());
    assert_eq!(plan.refusals.len(), 1);
}

/// A harness that is not installed is reported, not quietly skipped.
#[test]
fn an_uninstalled_harness_is_refused() {
    let home = FakeHome::new("notinstalled");
    std::fs::remove_dir_all(home.path(".gemini")).unwrap();

    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["gemini-cli".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    assert!(plan.steps.is_empty());
    assert!(plan.refusals[0].reason.contains("not installed"));
}

/// NF-1.6: a config that does not parse is never written to.
#[test]
fn an_unparseable_config_is_refused() {
    let home = FakeHome::new("corrupt");
    std::fs::write(home.path(".codex/config.toml"), "this is not [[[ valid toml").unwrap();

    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    assert!(plan.steps.is_empty());
    assert!(plan.refusals[0].reason.contains("could not be parsed"));

    // And the corrupt file is untouched.
    assert_eq!(
        std::fs::read_to_string(home.path(".codex/config.toml")).unwrap(),
        "this is not [[[ valid toml"
    );
}

/// PROVIDER-SPEC §6 rule 3, end to end: updating a server must not delete the
/// harness-specific field the user set by hand.
#[test]
fn redeploying_preserves_hand_edited_harness_fields() {
    let home = FakeHome::new("preserve");
    std::fs::write(
        home.path(".codex/config.toml"),
        "model = \"gpt-5\"\n\n[mcp_servers.obsidian]\ncommand = \"old\"\nstartup_timeout_sec = 120\n",
    )
    .unwrap();

    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    inception_core::plan::apply(&plan);

    let text = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    assert!(text.contains("command = \"uvx\""), "not updated:\n{text}");
    assert!(
        text.contains("startup_timeout_sec = 120"),
        "hand-set field was deleted:\n{text}"
    );
}

/// Applying the same plan twice must not churn the file.
#[test]
fn a_second_identical_deploy_is_a_no_op() {
    let home = FakeHome::new("idempotent");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let make_plan = || {
        inception_core::plan::mcp_deploy(
            &providers,
            &["codex".to_string()],
            &obsidian_server(),
            Scope::User,
            &LiteralsOnly,
        )
    };

    inception_core::plan::apply(&make_plan());
    let after_first = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();

    let outcomes = inception_core::plan::apply(&make_plan());
    assert_eq!(outcomes[0].result.as_ref().unwrap(), "already up to date");
    assert_eq!(
        std::fs::read_to_string(home.path(".codex/config.toml")).unwrap(),
        after_first
    );
}

/// Cross-harness fields that cannot travel are reported in the preview (F-5.5).
#[test]
fn dropped_fields_are_named_in_the_preview() {
    let home = FakeHome::new("dropped");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let mut server = obsidian_server();
    server.source_provider = "codex".into();
    server.extra.insert("startup_timeout_sec", inception_core::Value::Int(120));

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["opencode".to_string()],
        &server,
        Scope::User,
        &LiteralsOnly,
    );
    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(preview.contains("dropped"), "preview:\n{preview}");
    assert!(preview.contains("startup_timeout_sec"), "preview:\n{preview}");
}

/// The preview must not write anything.
#[test]
fn preview_does_not_touch_disk() {
    let home = FakeHome::new("preview");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let before = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(!preview.is_empty());
    assert_eq!(
        std::fs::read_to_string(home.path(".codex/config.toml")).unwrap(),
        before
    );
}

/// **Regression test for a real leak.** The emitter resolves secrets to their
/// literal values because that is what gets written to disk — and the first
/// wiring of the preview rendered that same value, printing a live API key to
/// the terminal. The preview must mask credential-shaped keys while leaving the
/// rest of the environment visible (F-7.4).
#[test]
fn preview_masks_credentials_but_shows_ordinary_env_vars() {
    use inception_core::mcp::Secret;

    let home = FakeHome::new("redact");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let secret_value = "90e6ec080b81c2c7dca3d2371a588e5662e3ea9436fe5b6dda1224110251aaaa";
    let mut server = obsidian_server();
    server
        .env
        .push(("OBSIDIAN_API_KEY".into(), Secret::Literal(secret_value.into())));

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &server,
        Scope::User,
        &LiteralsOnly,
    );
    let preview = inception_core::plan::preview(&plan).unwrap();

    assert!(
        !preview.contains(secret_value),
        "SECRET LEAKED INTO PREVIEW:\n{preview}"
    );
    assert!(preview.contains("redacted"), "preview:\n{preview}");
    // Non-secret env vars are still shown — hiding everything would make the
    // preview useless.
    assert!(preview.contains("127.0.0.1"), "preview:\n{preview}");

    // ...but the value that actually reaches disk is the real one.
    inception_core::plan::apply(&plan);
    let written = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    assert!(written.contains(secret_value), "the real value must be written");
    assert!(!written.contains("redacted"));
}

/// F-7.4: a secret value must never appear in a preview.
#[test]
fn preview_never_renders_a_secret_value() {
    let home = FakeHome::new("secret");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let mut server = obsidian_server();
    server.env.push((
        "OBSIDIAN_API_KEY".into(),
        inception_core::mcp::Secret::Ref("obsidian.obsidian_api_key".into()),
    ));

    // With no keychain behind it the emit must fail rather than write a
    // placeholder — which also means the value can never leak into a preview.
    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &server,
        Scope::User,
        &LiteralsOnly,
    );
    assert!(plan.steps.is_empty());
    assert!(plan.refusals[0].reason.contains("obsidian.obsidian_api_key"));
}

// ---------------------------------------------------------------------------
// Removal (F-5.7)
// ---------------------------------------------------------------------------

#[test]
fn removing_an_mcp_server_is_surgical() {
    let home = FakeHome::new("rmmcp");
    std::fs::write(
        home.path(".codex/config.toml"),
        "# keep this comment\nmodel = \"gpt-5\"\n\n[mcp_servers.keep]\ncommand = \"a\"\n\n\
         [mcp_servers.drop]\ncommand = \"b\"\n",
    )
    .unwrap();

    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan =
        inception_core::plan::mcp_remove(&providers, &["codex".to_string()], "drop", Scope::User);
    assert_eq!(plan.steps.len(), 1);

    inception_core::plan::apply(&plan);
    let text = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();

    assert!(!text.contains("drop"));
    assert!(text.contains("[mcp_servers.keep]"), "sibling lost:\n{text}");
    assert!(text.contains("# keep this comment"), "comment lost:\n{text}");
    assert!(text.contains("model = \"gpt-5\""));
}

#[test]
fn removing_a_server_that_is_not_there_is_refused_not_silent() {
    let home = FakeHome::new("rmmissing");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan =
        inception_core::plan::mcp_remove(&providers, &["codex".to_string()], "ghost", Scope::User);
    assert!(plan.is_empty());
    assert_eq!(plan.refusals.len(), 1);
}

/// Deleting from a shared directory takes the skill away from every harness that
/// reads it. The plan must say so *before* the confirmation, not after.
#[test]
fn removing_from_a_shared_directory_names_every_affected_harness() {
    let home = FakeHome::new("rmshared");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    // Put a skill in the shared directory.
    let source = home.make_skill("shared-skill");
    let plan = inception_core::plan::skill_deploy(
        &providers,
        &["codex".to_string()],
        &source,
        Scope::User,
    );
    inception_core::plan::apply(&plan);
    assert!(home.path(".agents/skills/shared-skill").is_dir());

    // Removing it via Codex affects everyone reading ~/.agents/skills.
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::skill_remove(
        &providers,
        &["codex".to_string()],
        "shared-skill",
        Scope::User,
    );
    assert_eq!(plan.steps.len(), 1);

    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(preview.contains("shared directory"), "preview:\n{preview}");
    for id in ["codex", "opencode", "gemini-cli", "copilot-cli"] {
        assert!(preview.contains(id), "{id} not named in preview:\n{preview}");
    }
    assert!(!preview.contains("claude-code"), "Claude Code does not read it");

    inception_core::plan::apply(&plan);
    assert!(!home.path(".agents/skills/shared-skill").exists());
}

/// A skill deployed to Claude Code's private directory affects only it.
#[test]
fn removing_from_a_private_directory_affects_one_harness() {
    let home = FakeHome::new("rmprivate");
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let source = home.make_skill("private-skill");
    let plan = inception_core::plan::skill_deploy(
        &providers,
        &["claude-code".to_string()],
        &source,
        Scope::User,
    );
    inception_core::plan::apply(&plan);

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::skill_remove(
        &providers,
        &["claude-code".to_string()],
        "private-skill",
        Scope::User,
    );
    let step = match &plan.steps[0] {
        Step::SkillRemove(r) => r,
        other => panic!("expected a skill removal, got {other:?}"),
    };
    assert_eq!(step.providers, vec!["claude-code"]);
}

/// Deploy then remove should leave the config as it started.
#[test]
fn deploy_then_remove_restores_the_original_config() {
    let home = FakeHome::new("roundtrip");
    let before = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();

    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    let plan = inception_core::plan::mcp_deploy(
        &providers,
        &["codex".to_string()],
        &obsidian_server(),
        Scope::User,
        &LiteralsOnly,
    );
    inception_core::plan::apply(&plan);

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::mcp_remove(
        &providers,
        &["codex".to_string()],
        "obsidian",
        Scope::User,
    );
    inception_core::plan::apply(&plan);

    let after = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    // Whitespace around a removed table can legitimately differ; content must not.
    assert_eq!(
        after.replace("\n\n", "\n").trim(),
        before.replace("\n\n", "\n").trim(),
        "config did not return to its original content"
    );
}

fn _unused(_: &Path) {}
