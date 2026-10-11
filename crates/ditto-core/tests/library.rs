//! The library (Q-7) — a location, not a mode.
//!
//! These tests exist to pin one claim: switching the library off does not
//! select a second set of rules, it removes one directory from the scan. Every
//! other test in this crate runs with `library = None`, which *is* the
//! switched-off state, so the two configurations are covered by one suite.

use ditto_core::plan::{Plan, Step};
use ditto_core::spec::Scope;
use ditto_core::{PathContext, Registry};
use std::path::{Path, PathBuf};

/// A temporary home containing all five harnesses, removed on drop.
struct FakeHome(PathBuf);

impl FakeHome {
    fn new(tag: &str) -> FakeHome {
        let dir = std::env::temp_dir().join(format!(
            "ditto-library-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for sub in [
            ".claude",
            ".claude/skills",
            ".codex",
            ".config/opencode",
            ".config/opencode/skills",
            ".gemini",
            ".copilot",
            ".agents/skills",
            ".ditto/library",
        ] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join(".claude.json"), "{}\n").unwrap();
        std::fs::write(dir.join(".codex/config.toml"), "model = \"gpt-5\"\n").unwrap();
        std::fs::write(
            dir.join(".config/opencode/opencode.jsonc"),
            "{\n  \"mcp\": {}\n}\n",
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

    fn library(&self) -> PathBuf {
        self.path(".ditto/library")
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A skill directory with the given description, so two copies can differ.
fn put_skill(dir: &Path, name: &str, description: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\nBody.\n"),
    )
    .unwrap();
    path
}

fn skill_steps(plan: &Plan) -> Vec<&ditto_core::plan::SkillWrite> {
    plan.steps
        .iter()
        .filter_map(|s| match s {
            Step::Skill(w) => Some(w),
            _ => None,
        })
        .collect()
}

fn all_targets() -> Vec<String> {
    ["claude-code", "codex", "copilot-cli", "gemini-cli", "opencode"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// **The point of the library.** Without one, `default_source` answers "which
/// copy is canonical?" with whatever came first in path order. With one, it
/// answers with the library — and that is the only thing the switch changes.
#[test]
fn the_library_decides_the_default_source_and_nothing_else_does() {
    let home = FakeHome::new("default");
    put_skill(&home.path(".agents/skills"), "shared-thing", "the harness copy");
    put_skill(&home.library(), "shared-thing", "the library copy");

    let registry = Registry::builtin();
    let found = ditto_core::detect(&registry, &home.ctx(), Scope::User).unwrap();

    // On: the library wins.
    let groups =
        ditto_core::inventory::skills(&found, Scope::User, Some(&home.library()));
    let group = groups.iter().find(|g| g.identity == "shared-thing").unwrap();
    let chosen = ditto_core::inventory::default_source(group).unwrap();
    assert!(chosen.library, "the library copy should be the default source");
    assert_eq!(chosen.path, home.library().join("shared-thing"));

    // Off: exactly the behaviour that shipped before the library existed. The
    // same code runs, over a smaller set of locations.
    let groups = ditto_core::inventory::skills(&found, Scope::User, None);
    let group = groups.iter().find(|g| g.identity == "shared-thing").unwrap();
    let chosen = ditto_core::inventory::default_source(group).unwrap();
    assert!(!chosen.library);
    assert!(chosen.path.starts_with(home.path(".agents/skills")));
}

/// Asking for a harness's version must never answer with ours. T-5.15 exists
/// because this question was answered wrongly once already.
#[test]
fn asking_for_a_harnesss_version_never_returns_the_library() {
    let home = FakeHome::new("notharness");
    put_skill(&home.path(".config/opencode/skills"), "whose", "opencode's own");
    put_skill(&home.library(), "whose", "the library's");

    let registry = Registry::builtin();
    let found = ditto_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups =
        ditto_core::inventory::skills(&found, Scope::User, Some(&home.library()));
    let group = groups.iter().find(|g| g.identity == "whose").unwrap();

    let opencode = ditto_core::inventory::source_for(group, "opencode").unwrap();
    assert!(!opencode.library);
    assert_eq!(
        opencode.path,
        home.path(".config/opencode/skills").join("whose")
    );
}

/// A deploy reads the library but never writes to it. Adopting a skill is a
/// decision, not a side effect of sending it somewhere.
#[test]
fn deploying_never_writes_into_the_library() {
    let home = FakeHome::new("nowrite");
    let source = put_skill(&home.library(), "from-library", "canonical");

    let registry = Registry::builtin();
    let found = ditto_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let plan = ditto_core::plan::skill_deploy(&found, &all_targets(), &source, Scope::User);

    assert!(
        !plan.steps.is_empty(),
        "deploying the library's copy should still reach the harnesses"
    );
    for write in skill_steps(&plan) {
        assert!(
            !write.dest.starts_with(home.library()),
            "a deploy wrote into the library: {}",
            write.dest.display()
        );
    }
}

/// The library belongs to the account. A project matrix listing every skill in
/// a thirty-skill library would bury a project holding two.
#[test]
fn the_library_is_not_a_project_scope_row_but_is_still_a_source() {
    let home = FakeHome::new("userscope");
    put_skill(&home.library(), "mine", "only in the library");

    let project = home.path("repo");
    std::fs::create_dir_all(project.join(".claude/skills")).unwrap();

    let registry = Registry::builtin();
    let ctx = home.ctx().with_project(&project);
    let found = ditto_core::detect(&registry, &ctx, Scope::Project).unwrap();
    let groups =
        ditto_core::inventory::skills(&found, Scope::Project, Some(&home.library()));
    assert!(
        groups.iter().all(|g| g.identity != "mine"),
        "the library leaked into project scope"
    );

    // Still reachable from there as a *source*, which is what makes moving a
    // skill out of the library and into a repository work.
    let plan = ditto_core::scope::transfer(
        &registry,
        &home.ctx(),
        &project,
        "mine",
        ditto_core::scope::Direction::IntoProject,
        &[],
        Some(&home.library()),
    )
    .unwrap();
    assert!(!plan.steps.is_empty());
    assert!(
        skill_steps(&plan)
            .iter()
            .all(|w| w.source.starts_with(home.library())),
        "a transfer into a project should source from the library"
    );
}

/// Import is an ordinary skill write, so it inherits the preview, the
/// replacement diff and `apply` instead of growing its own copies of them.
#[test]
fn importing_copies_into_the_library_and_is_idempotent() {
    let home = FakeHome::new("import");
    let source = put_skill(&home.path(".agents/skills"), "adopt-me", "from a harness");

    let plan = ditto_core::plan::skill_import(&source, &home.library());
    assert_eq!(skill_steps(&plan).len(), 1);
    let write = skill_steps(&plan)[0];
    assert_eq!(write.dest, home.library().join("adopt-me"));
    assert!(
        write.providers.is_empty(),
        "no harness reads the library, so the write serves none"
    );

    for outcome in ditto_core::plan::apply_with(
        &plan,
        &ditto_core::write::Options::with_backup_dir(home.path("backups")),
    ) {
        assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    }
    assert!(home.library().join("adopt-me/SKILL.md").is_file());

    // The same content again has nothing to do, and says so rather than
    // reporting a write that changes nothing.
    let again = ditto_core::plan::skill_import(&source, &home.library());
    assert!(again.is_empty());
    assert_eq!(again.refusals.len(), 1);
    assert!(again.refusals[0].reason.contains("already holds"));
}

/// Re-importing after an edit replaces the library copy, and the preview says
/// it destroys what is there — the guarantee T-3.23 gives harness copies.
#[test]
fn reimporting_an_edited_skill_announces_the_replacement() {
    let home = FakeHome::new("reimport");
    put_skill(&home.library(), "edited", "the old description");
    let source = put_skill(&home.path(".agents/skills"), "edited", "a new description");

    let plan = ditto_core::plan::skill_import(&source, &home.library());
    let write = skill_steps(&plan)[0];
    assert!(write.replaces_divergent);

    let preview = ditto_core::plan::preview(&plan).unwrap();
    assert!(
        preview.contains("REPLACES an existing copy that differs"),
        "{preview}"
    );
    // A library write serves no harness, and a bare "serves:" line reads as a
    // bug rather than as a fact.
    assert!(!preview.contains("serves:"), "{preview}");
}

/// A library copy that disagrees with the harnesses is drift, and the report
/// has to name the library — "differs between codex" says nothing about what
/// the other side even is.
#[test]
fn doctor_names_the_library_when_it_is_the_one_that_differs() {
    let home = FakeHome::new("doctor");
    put_skill(&home.library(), "split", "the library's idea");
    put_skill(&home.path(".agents/skills"), "split", "the harness's idea");

    let registry = Registry::builtin();
    let found = ditto_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let findings =
        ditto_core::doctor::check(&found, Scope::User, Some(&home.library()));

    let drift = findings
        .iter()
        .find(|f| f.message.contains("'split'"))
        .expect("drift between the library and a harness should be reported");
    assert!(drift.message.contains("your library"), "{}", drift.message);
}

/// Recency is read from disk and reported, but it must never be what picks the
/// winner: here the *newest* copy is the one a deploy would overwrite.
#[test]
fn recency_is_reported_but_does_not_choose() {
    let home = FakeHome::new("recency");
    put_skill(&home.library(), "timed", "the library's");
    // Written second, so it is the newer file on every filesystem that records
    // modification times at all.
    let harness = put_skill(&home.path(".agents/skills"), "timed", "the harness's");

    let registry = Registry::builtin();
    let found = ditto_core::detect(&registry, &home.ctx(), Scope::User).unwrap();
    let groups =
        ditto_core::inventory::skills(&found, Scope::User, Some(&home.library()));
    let group = groups.iter().find(|g| g.identity == "timed").unwrap();

    // Every copy carries a time, so the picker has something to show.
    assert!(
        group.locations.iter().all(|l| l.modified().is_some()),
        "a readable skill directory should report when it last changed"
    );

    // And the newer copy is not thereby the winner.
    let newest = group
        .locations
        .iter()
        .max_by_key(|l| l.modified())
        .unwrap();
    assert_eq!(newest.path, harness, "test setup: the harness copy is newer");

    let chosen = ditto_core::inventory::default_source(group).unwrap();
    assert!(
        chosen.library,
        "recency must not override the library as the default source"
    );
}
