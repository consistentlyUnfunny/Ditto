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

/// Apply a plan with backups redirected into the fake home.
///
/// Without this, every test that writes a file drops a backup into the *real*
/// `~/.inception/backups`, where it is indistinguishable from one the user
/// might actually need.
fn run(home: &FakeHome, plan: &Plan) -> Vec<inception_core::plan::Outcome> {
    inception_core::plan::apply_with(
        plan,
        &inception_core::write::Options::with_backup_dir(home.path("backups")),
    )
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
    let outcomes = run(&home, &plan);

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
    // The shared diff renderer writes "  + text" — sign, then a space.
    assert!(preview.contains("+ [mcp_servers.obsidian]"), "preview:\n{preview}");

    let codex_before = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    let outcomes = run(&home, &plan);
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
    let outcomes = run(&home, &plan);

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
    run(&home, &plan);

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
    run(&home, &plan);

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

    run(&home, &make_plan());
    let after_first = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();

    let outcomes = run(&home, &make_plan());
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

    // Fully synthetic. An earlier version of this fixture was derived from a
    // real key with only the last four characters changed, which would have
    // committed 60 of its 64 characters to the repository.
    let secret_value = "not-a-real-key-0000000000000000000000000000000000000000000000000";
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
    run(&home, &plan);
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
// Convergence — a deploy must resolve drift, not add to it
// ---------------------------------------------------------------------------

/// **Regression test for a bug found by using the app.**
///
/// A harness usually reads more than one skill directory. Deploying into the
/// shared one while a *different* copy sits in that harness's private directory
/// leaves it seeing two versions of the same skill — strictly worse than the
/// drift the deploy was meant to fix. The first version of the planner did
/// exactly that: after "deploy to all", `karpathy-coder` existed in four places
/// with two different hashes, and the matrix reported everything as drifted.
#[test]
fn deploying_overwrites_a_divergent_copy_the_target_also_reads() {
    let home = FakeHome::new("converge");
    let registry = Registry::builtin();

    // OpenCode's private directory holds an older, different version.
    let private = home.path(".config/opencode/skills/karpathy-coder");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(
        private.join("SKILL.md"),
        "---\nname: karpathy-coder\ndescription: The OLD text.\n---\n\nOld body.\n",
    )
    .unwrap();

    // The source lives in the shared directory.
    let source = home.path(".agents/skills/karpathy-coder");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: karpathy-coder\ndescription: The NEW text.\n---\n\nNew body.\n",
    )
    .unwrap();

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan =
        inception_core::plan::skill_deploy(&providers, &all_targets(), &source, Scope::User);

    // The private copy must be in the write set, flagged as a replacement.
    let replacing = skill_steps(&plan)
        .into_iter()
        .find(|w| w.dest == private)
        .expect("the divergent private copy must be overwritten");
    assert!(replacing.replaces_divergent);
    assert!(
        replacing.reason.contains("shadow"),
        "the reason should explain why: {}",
        replacing.reason
    );

    // Copying the source onto itself is not a write.
    assert!(
        !skill_steps(&plan).iter().any(|w| w.dest == source),
        "should not copy a directory onto itself"
    );

    // The preview shows what is about to be destroyed.
    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(preview.contains("REPLACES"), "preview:\n{preview}");
    assert!(preview.contains("- description: The OLD text."), "preview:\n{preview}");
    assert!(preview.contains("+ description: The NEW text."), "preview:\n{preview}");

    // And after applying, every copy agrees.
    run(&home, &plan);
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let group = groups.iter().find(|g| g.identity == "karpathy-coder").unwrap();
    assert_eq!(
        group.distinct_hashes().len(),
        1,
        "deploy should converge every copy, found: {:#?}",
        group.locations.iter().map(|l| &l.path).collect::<Vec<_>>()
    );
}

/// A copy that already agrees is left alone — no needless rewrite.
#[test]
fn deploying_does_not_rewrite_copies_that_already_match() {
    let home = FakeHome::new("nochurn");
    let registry = Registry::builtin();

    let body = "---\nname: same\ndescription: Identical.\n---\n\nBody.\n";
    for rel in [".config/opencode/skills/same", ".agents/skills/same"] {
        let dir = home.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::skill_deploy(
        &providers,
        &all_targets(),
        &home.path(".agents/skills/same"),
        Scope::User,
    );

    assert!(
        !skill_steps(&plan)
            .iter()
            .any(|w| w.dest == home.path(".config/opencode/skills/same")),
        "an identical copy should not be rewritten"
    );
}

/// Drift must be explained by the copies that actually differ, not simply the
/// first two in path order — with three agreeing copies and one outlier, taking
/// "the first two" produced an empty diff.
#[test]
fn comparison_picks_copies_that_actually_differ() {
    let home = FakeHome::new("comparepick");
    let registry = Registry::builtin();

    let same = "---\nname: k\ndescription: Agreed.\n---\n\nBody.\n";
    for rel in [".agents/skills/k", ".claude/skills/k"] {
        let dir = home.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), same).unwrap();
    }
    let odd = home.path(".config/opencode/skills/k");
    std::fs::create_dir_all(&odd).unwrap();
    std::fs::write(
        odd.join("SKILL.md"),
        "---\nname: k\ndescription: The odd one out.\n---\n\nBody.\n",
    )
    .unwrap();

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let group = groups.iter().find(|g| g.identity == "k").unwrap();

    let comparison = inception_core::inventory::compare_copies(group)
        .expect("three copies means there is something to compare");
    assert!(
        inception_core::diff::has_changes(&comparison.lines),
        "comparison must land on the copies that differ, got an empty diff between {} and {}",
        comparison.left.path.display(),
        comparison.right.path.display()
    );
}

// ---------------------------------------------------------------------------
// Source selection (T-5.14, T-5.15)
// ---------------------------------------------------------------------------

/// Builds the shape the dev machine actually has: OpenCode reads both the shared
/// directory and its own, and the two disagree.
fn home_with_two_versions(tag: &str) -> (FakeHome, Registry) {
    let home = FakeHome::new(tag);
    for (rel, description) in [
        (".agents/skills/k", "The shared version."),
        (".config/opencode/skills/k", "OpenCode's own version."),
    ] {
        let dir = home.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: k\ndescription: {description}\n---\n\nBody.\n"),
        )
        .unwrap();
    }
    (home, Registry::builtin())
}

fn group_named<'a>(
    groups: &'a [inception_core::inventory::SkillGroup],
    name: &str,
) -> &'a inception_core::inventory::SkillGroup {
    groups.iter().find(|g| g.identity == name).expect("group")
}

/// **"OpenCode's version" must mean the copy OpenCode actually loads.**
///
/// OpenCode reads `~/.agents/skills` as well as its own directory, and the
/// shared one sorts first alphabetically — so a naive "first location serving
/// this provider" returned the shared copy, the opposite of what
/// `--from opencode` means.
#[test]
fn source_for_follows_the_harnesss_own_precedence() {
    let (home, registry) = home_with_two_versions("sourcepick");
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let group = group_named(&groups, "k");

    let chosen = inception_core::inventory::source_for(group, "opencode").unwrap();
    assert_eq!(chosen.path, home.path(".config/opencode/skills/k"));

    // Gemini has no private copy, so the shared one is genuinely its version.
    let gemini = inception_core::inventory::source_for(group, "gemini-cli").unwrap();
    assert_eq!(gemini.path, home.path(".agents/skills/k"));
}

/// **Precedence is per harness, not a shared-versus-private rule (T-6.7).**
///
/// Gemini reads `~/.agents/skills` *before* `~/.gemini/skills`, the reverse of
/// OpenCode. So when both hold a copy, "Gemini's version" is the shared one —
/// that is the text Gemini loads. Ranking private above shared answered this
/// backwards for one of the five harnesses.
#[test]
fn source_for_reads_precedence_per_harness_not_by_sharedness() {
    let home = FakeHome::new("sourceorder");
    for (rel, description) in [
        (".agents/skills/k", "The shared version."),
        (".gemini/skills/k", "Gemini's private copy."),
        (".config/opencode/skills/k", "OpenCode's own version."),
    ] {
        let dir = home.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---
name: k
description: {description}
---

Body.
"),
        )
        .unwrap();
    }
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let group = group_named(&groups, "k");

    // Same group, same two candidate kinds, opposite answers.
    assert_eq!(
        inception_core::inventory::source_for(group, "gemini-cli")
            .unwrap()
            .path,
        home.path(".agents/skills/k"),
        "gemini reads the shared directory first, so that is the copy it loads"
    );
    assert_eq!(
        inception_core::inventory::source_for(group, "opencode")
            .unwrap()
            .path,
        home.path(".config/opencode/skills/k"),
        "opencode reads its own directory first"
    );
}

/// A deprecated directory is a last resort, never the preferred source.
#[test]
fn source_for_avoids_deprecated_locations() {
    let home = FakeHome::new("sourcedep");
    for rel in [".codex/skills/k", ".agents/skills/k"] {
        let dir = home.path(rel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: k\ndescription: d.\n---\n\nB.\n").unwrap();
    }
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);

    let chosen =
        inception_core::inventory::source_for(group_named(&groups, "k"), "codex").unwrap();
    assert_eq!(
        chosen.path,
        home.path(".agents/skills/k"),
        "codex's only private directory is deprecated, so the shared one wins"
    );
}

/// Choosing a source changes which version survives — the whole point.
#[test]
fn choosing_the_source_decides_which_version_wins() {
    let (home, registry) = home_with_two_versions("sourcewins");
    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let group = group_named(&groups, "k");
    assert_eq!(inception_core::inventory::versions(group).len(), 2);

    let source = inception_core::inventory::source_for(group, "opencode").unwrap();
    let plan =
        inception_core::plan::skill_deploy(&providers, &all_targets(), &source.path, Scope::User);
    run(&home, &plan);

    // Every copy now holds OpenCode's text.
    for rel in [".agents/skills/k", ".config/opencode/skills/k", ".claude/skills/k"] {
        let text = std::fs::read_to_string(home.path(rel).join("SKILL.md")).unwrap();
        assert!(
            text.contains("OpenCode's own version."),
            "{rel} did not take the chosen version:\n{text}"
        );
    }

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    assert_eq!(inception_core::inventory::versions(group_named(&groups, "k")).len(), 1);
}

/// Versions group by content, not by location: 2 copies of one text and 1 of
/// another is a two-way choice, not a three-way one.
#[test]
fn versions_group_by_content() {
    let (home, registry) = home_with_two_versions("versions");
    // A third copy agreeing with the shared one.
    let extra = home.path(".claude/skills/k");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(
        extra.join("SKILL.md"),
        "---\nname: k\ndescription: The shared version.\n---\n\nBody.\n",
    )
    .unwrap();

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups = inception_core::inventory::skills(&providers, Scope::User, None);
    let versions = inception_core::inventory::versions(group_named(&groups, "k"));

    assert_eq!(versions.len(), 2, "three copies, two versions");
    let biggest = versions.iter().map(|v| v.len()).max().unwrap();
    assert_eq!(biggest, 2);
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

    run(&home, &plan);
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
    run(&home, &plan);
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

    run(&home, &plan);
    assert!(!home.path(".agents/skills/shared-skill").exists());
}

/// Nothing is truly private: `~/.claude/skills` serves OpenCode as well.
#[test]
fn removing_from_claude_code_also_affects_opencode() {
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
    run(&home, &plan);

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

    // `~/.claude/skills` is *not* private: OpenCode reads it too, alongside its
    // own directory and `~/.agents/skills`. So deleting from it takes the skill
    // away from OpenCode as well, and the plan has to say so before anyone
    // confirms. This assertion used to read `["claude-code"]`, which encoded a
    // wrong model of OpenCode rather than a wrong implementation.
    assert_eq!(step.providers, vec!["claude-code", "opencode"]);

    let preview = inception_core::plan::preview(&plan).unwrap();
    assert!(preview.contains("shared directory"), "preview:\n{preview}");
    assert!(preview.contains("opencode"), "preview:\n{preview}");
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
    run(&home, &plan);

    let providers = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = inception_core::plan::mcp_remove(
        &providers,
        &["codex".to_string()],
        "obsidian",
        Scope::User,
    );
    run(&home, &plan);

    let after = std::fs::read_to_string(home.path(".codex/config.toml")).unwrap();
    // Whitespace around a removed table can legitimately differ; content must not.
    assert_eq!(
        after.replace("\n\n", "\n").trim(),
        before.replace("\n\n", "\n").trim(),
        "config did not return to its original content"
    );
}

fn _unused(_: &Path) {}

// ---------------------------------------------------------------------------
// Instruction shadowing (T-6.8)
// ---------------------------------------------------------------------------

/// **A repo holding both AGENTS.md and CLAUDE.md gives one harness two briefs.**
///
/// Nothing inside a harness can see this: each just loads what it finds, and the
/// contradiction only surfaces as odd behaviour. It is exactly the class of
/// problem a cross-harness view exists to catch.
#[test]
fn doctor_flags_a_repo_that_holds_two_instruction_files_for_one_harness() {
    let home = FakeHome::new("instr");
    let project = home.path("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("AGENTS.md"), "# Agents\n\nUse tabs.\n").unwrap();
    std::fs::write(project.join("CLAUDE.md"), "# Claude\n\nUse spaces.\n").unwrap();

    let ctx = home.ctx().with_project(&project);
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &ctx, Scope::Project).unwrap();
    let findings = inception_core::doctor::check(&providers, Scope::Project, None);

    let hit = findings
        .iter()
        .find(|f| f.message.contains("standing instructions"))
        .expect("a harness reading both files must be reported");
    assert_eq!(hit.provider.as_deref(), Some("claude-code"));
    assert!(
        hit.message.contains("AGENTS.md") && hit.message.contains("CLAUDE.md"),
        "the message must name both files, got: {}",
        hit.message
    );

    // Codex reads only AGENTS.md, so it has nothing to be confused about.
    assert!(
        !findings
            .iter()
            .any(|f| f.provider.as_deref() == Some("codex")
                && f.message.contains("standing instructions")),
        "a harness that reads one of the two files must not be flagged"
    );
}

/// One file alone is the normal case and must stay silent.
#[test]
fn doctor_is_silent_when_a_repo_holds_one_instruction_file() {
    let home = FakeHome::new("instrone");
    let project = home.path("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("AGENTS.md"), "# Agents\n\nUse tabs.\n").unwrap();

    let ctx = home.ctx().with_project(&project);
    let registry = Registry::builtin();
    let providers = inception_core::detect(&registry, &ctx, Scope::Project).unwrap();
    let findings = inception_core::doctor::check(&providers, Scope::Project, None);

    assert!(
        !findings.iter().any(|f| f.message.contains("standing instructions")),
        "a single AGENTS.md is the convention working, not a problem"
    );
}

// ---------------------------------------------------------------------------
// Project scope (M6)
// ---------------------------------------------------------------------------

/// **A project deploy must not commit five skill directories to a repo.**
///
/// This is what the M6 research pass was for. Before the specs were corrected
/// they declared only each harness's own project directory, so serving all five
/// meant five copies of every skill in the working tree. The shared
/// `.agents/skills` and `.claude/skills` between them reach everyone.
#[test]
fn a_project_deploy_writes_two_directories_not_five() {
    let home = FakeHome::new("projwrite");
    let project = home.path("repo");
    std::fs::create_dir_all(&project).unwrap();
    let source = home.make_skill("shared-thing");

    let ctx = home.ctx().with_project(&project);
    let registry = Registry::builtin();
    let found = inception_core::detect(&registry, &ctx, Scope::Project).unwrap();
    let targets: Vec<String> = found.iter().map(|p| p.id().to_string()).collect();
    let plan = inception_core::plan::skill_deploy(&found, &targets, &source, Scope::Project);

    assert_eq!(
        plan.steps.len(),
        2,
        "expected two directories to cover five harnesses, got: {:?}",
        plan.steps
            .iter()
            .map(|s| format!("{s:?}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        plan.providers_served().len(),
        5,
        "two writes must still serve every harness"
    );
}

/// Every write inside a project is a write a teammate will see (T-6.6).
#[test]
fn project_writes_are_reported_as_landing_inside_the_repository() {
    let home = FakeHome::new("projgit");
    let project = home.path("repo");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let source = home.make_skill("tracked");

    let ctx = home.ctx().with_project(&project);
    let registry = Registry::builtin();
    let found = inception_core::detect(&registry, &ctx, Scope::Project).unwrap();
    let targets: Vec<String> = found.iter().map(|p| p.id().to_string()).collect();
    let plan = inception_core::plan::skill_deploy(&found, &targets, &source, Scope::Project);

    for step in &plan.steps {
        let dest = match step {
            inception_core::plan::Step::Skill(s) => s.dest.clone(),
            other => panic!("expected skill steps, got {other:?}"),
        };
        assert_eq!(
            inception_core::projects::git_root(&dest).as_deref(),
            Some(project.as_path()),
            "{} is inside the repo and must be reported as such",
            dest.display()
        );
    }

    // The same skill at user scope is not a write into *this* repo. It can still
    // land in a repo of its own — a dotfiles home is a working tree, and the dev
    // machine has exactly that — so the claim is about which repo, not whether.
    let user = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let user_plan = inception_core::plan::skill_deploy(&user, &targets, &source, Scope::User);
    for step in &user_plan.steps {
        if let inception_core::plan::Step::Skill(s) = step {
            assert_ne!(
                inception_core::projects::git_root(&s.dest).as_deref(),
                Some(project.as_path()),
                "{} is a user-scope path and must not be attributed to the project repo",
                s.dest.display()
            );
        }
    }
}

/// Markers come from the specs, so adding a provider adds its markers for free.
#[test]
fn project_markers_are_derived_from_the_specs() {
    let home = FakeHome::new("projmark");
    let project = home.path("repo");
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::write(project.join("AGENTS.md"), "# Agents\n").unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();

    let registry = Registry::builtin();
    let found = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let described = inception_core::projects::describe(&project, &found, &[]);

    assert_eq!(described.name, "repo");
    assert!(described.markers.contains(&".claude".to_string()));
    assert!(described.markers.contains(&"AGENTS.md".to_string()));
    assert!(
        !described.markers.contains(&"src".to_string()),
        "only things a harness reads are markers, got {:?}",
        described.markers
    );
    assert!(
        described.sources.is_empty(),
        "a hand-picked directory has no recording harness"
    );
}

/// **An empty repository must still offer somewhere to deploy.**
///
/// Installed is a fact about the machine. Judged per scope, a fresh project
/// reports no harnesses at all and the user is left staring at an empty screen
/// with no way forward — the state every new project starts in.
#[test]
fn a_fresh_project_still_sees_the_harnesses_on_this_machine() {
    let home = FakeHome::new("projfresh");
    let project = home.path("empty-repo");
    std::fs::create_dir_all(&project).unwrap();

    let registry = Registry::builtin();
    let found =
        inception_core::detect(&registry, &home.ctx().with_project(&project), Scope::Project)
            .unwrap();

    let installed: Vec<&str> = found.iter().filter(|p| p.installed).map(|p| p.id()).collect();
    assert_eq!(
        installed.len(),
        5,
        "every harness on the machine is a valid target in a new project, got {installed:?}"
    );

    // And nothing is claimed to be *in* the project, because nothing is.
    assert!(
        inception_core::inventory::skills(&found, Scope::Project, None).is_empty(),
        "an empty repo holds no skills"
    );
}

/// **A transfer is a deploy whose source sits in the other scope (F-5.9).**
///
/// Asserted round-trip, because the two directions are not symmetric: user scope
/// has private per-harness directories that project scope does not.
#[test]
fn a_skill_transfers_into_a_project_and_back_out_again() {
    use inception_core::scope::{Direction, transfer};

    let home = FakeHome::new("xfer");
    let project = home.path("repo");
    std::fs::create_dir_all(&project).unwrap();

    // Put the skill at user scope the way a deploy would.
    let source = home.make_skill("portable");
    let registry = Registry::builtin();
    let user = inception_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let targets: Vec<String> = user.iter().map(|p| p.id().to_string()).collect();
    run(
        &home,
        &inception_core::plan::skill_deploy(&user, &targets, &source, Scope::User),
    );

    // Into the project.
    let plan = transfer(
        &registry,
        &home.ctx(),
        &project,
        "portable",
        Direction::IntoProject,
        &[],
        None,
    )
    .unwrap();
    assert_eq!(plan.providers_served().len(), 5, "every harness should see it");
    run(&home, &plan);
    assert!(project.join(".agents/skills/portable/SKILL.md").is_file());
    assert!(project.join(".claude/skills/portable/SKILL.md").is_file());

    // Bundled resources travel too — a skill is a directory, not a file.
    assert!(project.join(".agents/skills/portable/scripts/helper.sh").is_file());

    // Running it again is a no-op, not a second set of writes.
    let again = transfer(
        &registry,
        &home.ctx(),
        &project,
        "portable",
        Direction::IntoProject,
        &[],
        None,
    )
    .unwrap();
    assert!(
        again.steps.is_empty(),
        "a transfer that has already happened must converge, got {} writes",
        again.steps.len()
    );

    // And back out: the project copy becomes the user's.
    let back = transfer(
        &registry,
        &home.ctx(),
        &project,
        "portable",
        Direction::IntoAccount,
        &[],
        None,
    )
    .unwrap();
    assert!(
        back.steps.is_empty(),
        "identical copies in both scopes means nothing to do"
    );
}

/// A transfer of something that is not there names the scope it looked in.
#[test]
fn transferring_something_absent_says_where_it_looked() {
    use inception_core::scope::{Direction, transfer};

    let home = FakeHome::new("xfermiss");
    let project = home.path("repo");
    std::fs::create_dir_all(&project).unwrap();

    let error = transfer(
        &Registry::builtin(),
        &home.ctx(),
        &project,
        "nothing-like-this",
        Direction::IntoProject,
        &[],
        None,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("user scope"),
        "the message must say which scope was searched, got: {error}"
    );
}
