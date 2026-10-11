//! `inception` — command-line interface.
//!
//! A thin shell over `inception-core`. All logic lives in the engine so the GUI
//! (M4) and the CLI stay in lockstep (F-9.1).

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use inception_core::spec::Scope;
use inception_core::{PathContext, Registry, Settings};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Parser)]
#[command(
    name = "inception",
    version,
    about = "Manage skills and MCP servers across AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show which harnesses are installed and where their config lives.
    Status {
        /// Inspect project scope for this directory instead of user scope.
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Unified inventory across every detected harness.
    List {
        /// What to list.
        #[arg(value_enum, default_value_t = What::Skills)]
        what: What,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List the projects your harnesses have recorded, plus any you added.
    Projects {
        /// Add a directory to the list. Harness records cover where you already
        /// work; this covers a repo you have not opened an agent in yet.
        #[arg(long, value_name = "DIR")]
        add: Option<PathBuf>,
        /// Drop a directory you added. A harness's own record is not ours to
        /// remove — it would reappear on the next scan.
        #[arg(long, value_name = "DIR")]
        forget: Option<PathBuf>,
    },
    /// Copy a skill between your account and a project (F-5.9).
    Transfer {
        /// Skill name, as shown by `list`.
        name: String,
        /// Where it should end up.
        #[arg(long, value_enum)]
        to: Where,
        /// The project. Required — a transfer needs two scopes.
        #[arg(long)]
        project: PathBuf,
        /// Comma-separated provider ids, or "all".
        #[arg(long, default_value = "all")]
        targets: String,
        #[arg(long)]
        yes: bool,
    },
    /// Report skills that exist in several places with differing content.
    Drift {
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Copy a skill, or an MCP server, to other harnesses.
    Deploy {
        /// Skill name or MCP server name, as shown by `list`.
        name: String,
        /// Comma-separated provider ids, or "all".
        #[arg(long, default_value = "all")]
        to: String,
        /// Which harness's copy to deploy *from*, when copies disagree.
        /// Without it, path order decides — which is no choice at all.
        #[arg(long)]
        from: Option<String>,
        /// Apply without asking. The diff is still printed.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Adopt a skill into your library, so deploys have a canonical source.
    Import {
        /// Skill name, as shown by `list`.
        name: String,
        /// Which harness's copy to adopt, when copies disagree.
        #[arg(long)]
        from: Option<String>,
        /// Apply without asking. The diff is still printed.
        #[arg(long)]
        yes: bool,
    },
    /// Remove a skill or MCP server from harnesses.
    Remove {
        name: String,
        /// Comma-separated provider ids, or "all".
        #[arg(long, default_value = "all")]
        from: String,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Check configuration health: deprecated paths, plaintext secrets, drift.
    Doctor {
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// List loaded provider specs and what they support.
    Providers,
    /// Validate provider specs (built-in, or a file you pass).
    Validate {
        /// Path to a .provider.toml. Omit to check the built-in specs.
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum What {
    Skills,
    Mcp,
}

#[derive(Clone, Copy, ValueEnum)]
enum Where {
    /// Into the project, where your team sees it.
    Project,
    /// Out to your account, where every project sees it.
    Account,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { project } => status(project),
        Command::List { what, project } => match what {
            What::Skills => list_skills(project),
            What::Mcp => list_mcp(project),
        },
        Command::Projects { add, forget } => projects(add, forget),
        Command::Transfer { name, to, project, targets, yes } => {
            transfer(name, to, project, targets, yes)
        }
        Command::Import { name, from, yes } => import(name, from, yes),
        Command::Drift { project } => drift(project),
        Command::Deploy { name, to, from, yes, project } => deploy(name, to, from, yes, project),
        Command::Doctor { project } => doctor(project),
        Command::Remove { name, from, yes, project } => remove(name, from, yes, project),
        Command::Providers => providers(),
        Command::Validate { path } => validate(path),
    }
}

/// Shared setup: load specs, resolve scope, probe the machine.
fn survey(project: Option<PathBuf>) -> Result<(Registry, PathContext, Scope, Settings)> {
    let registry = Registry::load();
    report_rejected(&registry);
    let mut ctx = PathContext::detect()?;
    let scope = match &project {
        Some(dir) => {
            ctx = ctx.with_project(std::path::absolute(dir)?);
            Scope::Project
        }
        None => Scope::User,
    };
    Ok((registry, ctx, scope, Settings::load()?))
}

/// Who holds a copy, for a line that lists one.
///
/// The library is read by no harness, so its provider list is empty and would
/// otherwise print as nothing at all.
fn holder(location: &inception_core::inventory::SkillLocation) -> String {
    if location.library {
        "your library".to_string()
    } else {
        location.providers.join(", ")
    }
}

/// A recency label, or nothing when the filesystem would not say.
///
/// Recency is shown while choosing and never used to choose — see
/// `inception_core::when` for why.
fn changed(at: Option<std::time::SystemTime>) -> String {
    match at {
        Some(t) => format!("  changed {}", inception_core::when::relative(t, SystemTime::now())),
        None => String::new(),
    }
}

/// Projects a harness has recorded — a better list than anything we could get
/// by crawling directories, because it is where the user actually works — plus
/// any they added by hand, which is the only way a repo with no agent history
/// can get on the list at all.
fn projects(add: Option<PathBuf>, forget: Option<PathBuf>) -> Result<()> {
    let registry = Registry::load();
    let ctx = PathContext::detect()?;
    let mut settings = inception_core::Settings::load()?;

    if let Some(dir) = add {
        let dir = std::path::absolute(&dir)?;
        if !dir.is_dir() {
            anyhow::bail!("{} is not a directory", dir.display());
        }
        if settings.add_project(&dir) {
            let path = settings.save()?;
            println!("Added {}
  recorded in {}
", dir.display(), path.display());
        } else {
            println!("{} is already on the list.
", dir.display());
        }
    }

    if let Some(dir) = forget {
        let dir = std::path::absolute(&dir)?;
        if settings.forget_project(&dir) {
            settings.save()?;
            println!("Forgot {}
", dir.display());
        } else {
            // Say *why* rather than reporting a silent no-op: the likeliest
            // reason is that a harness recorded it, and that is not ours to
            // remove.
            println!(
                "{} was not on your list. If it still shows up, a harness recorded it — 
                 clear it in that harness's own config.
",
                dir.display()
            );
        }
    }

    let found = inception_core::detect(&registry, &ctx, Scope::User)?;
    let projects = inception_core::projects::discover(&found, &settings.projects);

    if projects.is_empty() {
        println!("No projects yet. Add one with `inception projects --add <dir>`.");
        return Ok(());
    }

    for project in &projects {
        let tags = {
            let mut tags = Vec::new();
            if project.is_git {
                tags.push("git".to_string());
            }
            tags.extend(project.markers.iter().cloned());
            tags
        };
        println!("{}", project.name);
        println!("  {}", project.path.display());
        if project.sources.is_empty() {
            println!("  added by you; no harness has been used here yet");
        } else {
            println!(
                "  known to {}{}",
                project.sources.join(", "),
                if project.added { "  (also on your list)" } else { "" }
            );
        }
        if !tags.is_empty() {
            println!("  has {}", tags.join(", "));
        }
    }
    println!();
    println!(
        "{} project{}. Pass --project <path> to any command to work in one.",
        projects.len(),
        if projects.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

/// Move a skill between scopes. Same planner and preview as `deploy`, because
/// a transfer is a deploy whose source sits in the other scope (F-5.9).
fn transfer(
    name: String,
    to: Where,
    project: PathBuf,
    targets: String,
    yes: bool,
) -> Result<()> {
    use inception_core::scope::Direction;

    let registry = Registry::load();
    let ctx = PathContext::detect()?;
    let settings = Settings::load()?;
    let project = std::path::absolute(&project)?;
    let direction = match to {
        Where::Project => Direction::IntoProject,
        Where::Account => Direction::IntoAccount,
    };
    let targets: Vec<String> = if targets == "all" {
        Vec::new()
    } else {
        targets.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    let plan =
        inception_core::scope::transfer(
            &registry,
            &ctx,
            &project,
            &name,
            direction,
            &targets,
            settings.library().as_deref(),
        )?;

    if plan.is_empty() && plan.refusals.is_empty() {
        println!("Nothing to do — '{name}' is already there.");
        return Ok(());
    }

    print!("{}", inception_core::plan::preview(&plan)?);
    if plan.is_empty() {
        std::process::exit(1);
    }
    println!(
        "{} write(s), serving: {}",
        plan.steps.len(),
        plan.providers_served().join(", ")
    );

    if !yes && !confirm()? {
        println!("Aborted. Nothing was written.");
        return Ok(());
    }

    let mut failed = false;
    for outcome in inception_core::plan::apply(&plan) {
        match &outcome.result {
            Ok(message) => println!("ok    {} — {message}", outcome.target.display()),
            Err(e) => {
                failed = true;
                eprintln!("FAIL  {} — {e}", outcome.target.display());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn list_skills(project: Option<PathBuf>) -> Result<()> {
    let (registry, ctx, scope, settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;
    let groups = inception_core::inventory::skills(&found, scope, settings.library().as_deref());

    if groups.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    print_skill_row("SKILL", "STATUS", "AVAILABLE TO");
    for group in &groups {
        // `providers()` lists harnesses only. A skill that is in the library
        // and nowhere else would otherwise show a blank "available to" column,
        // which reads as "nothing can see this" rather than "not deployed yet".
        let mut seen_by = group.providers().join(", ");
        if group.locations.iter().any(|l| l.library) {
            seen_by = if seen_by.is_empty() {
                "library only".to_string()
            } else {
                format!("{seen_by}  (+ library)")
            };
        }
        print_skill_row(&truncate(&group.identity, 25), group.status.label(), &seen_by);
        for location in &group.locations {
            let note = match &location.entry.state {
                inception_core::SkillState::Ok(_) if location.shared => "  (shared dir)",
                inception_core::SkillState::Ok(_) => "",
                inception_core::SkillState::MissingSkillMd => "  !! no SKILL.md",
                inception_core::SkillState::Invalid(_) => "  !! unreadable",
            };
            let deprecated = if location.deprecated { "  !! deprecated path" } else { "" };
            let mine = if location.library { "  (your library)" } else { "" };
            println!(
                "    {}{}{}{}{}",
                location.path.display(),
                note,
                deprecated,
                mine,
                changed(location.modified())
            );
        }
    }

    let problems = groups
        .iter()
        .filter(|g| g.status == inception_core::inventory::DriftStatus::Problem)
        .count();
    let drifted = groups
        .iter()
        .filter(|g| g.status == inception_core::inventory::DriftStatus::Drifted)
        .count();
    println!(
        "\n{} skill(s); {} drifted, {} with problems.",
        groups.len(),
        drifted,
        problems
    );
    Ok(())
}

/// `inception import <name>` — copy a harness's skill into `~/.inception/library`.
///
/// The library is the answer to "which copy is canonical?" (Q-7). Nothing else
/// writes to it: a deploy reads it but never fills it, so adopting a skill stays
/// a decision rather than a side effect.
fn import(name: String, from: Option<String>, yes: bool) -> Result<()> {
    let (registry, ctx, scope, settings) = survey(None)?;

    let Some(library) = settings.library() else {
        anyhow::bail!(
            "the library is switched off — remove `use_library = false` from {}",
            inception_core::settings::settings_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/.inception/config.toml".to_string())
        );
    };

    let found = inception_core::detect(&registry, &ctx, scope)?;
    let groups =
        inception_core::inventory::skills(&found, scope, settings.library().as_deref());
    let group = groups
        .iter()
        .find(|g| g.identity == name)
        .ok_or_else(|| anyhow::anyhow!("no skill named '{name}'"))?;

    // Which copy to adopt. Without --from this is path order, which is exactly
    // the arbitrariness the library exists to end — so say which one won.
    let chosen = match &from {
        Some(provider) => inception_core::inventory::source_for(group, provider)
            .ok_or_else(|| anyhow::anyhow!("{provider} has no readable copy of '{name}'"))?,
        None => group
            .locations
            .iter()
            .find(|l| !l.library && l.entry.is_ok())
            .ok_or_else(|| anyhow::anyhow!("'{name}' has no readable copy to import"))?,
    };

    let versions = inception_core::inventory::versions(group);
    if versions.len() > 1 {
        println!(
            "'{name}' exists in {} versions; importing the copy from {}{}",
            versions.len(),
            holder(chosen),
            changed(chosen.modified())
        );
        println!("  {}", chosen.path.display());
        if from.is_none() {
            println!("  pass --from <harness> to adopt a different one");
        }
        println!();
    }

    let plan = inception_core::plan::skill_import(&chosen.path, &library);
    if plan.is_empty() && plan.refusals.is_empty() {
        println!("Nothing to do.");
        return Ok(());
    }

    print!("{}", inception_core::plan::preview(&plan)?);
    if plan.is_empty() {
        return Ok(());
    }
    if !yes && !confirm()? {
        println!("Aborted. Nothing was written.");
        return Ok(());
    }

    let mut failed = false;
    for outcome in inception_core::plan::apply(&plan) {
        match &outcome.result {
            Ok(message) => println!("ok    {} - {message}", outcome.target.display()),
            Err(e) => {
                failed = true;
                eprintln!("FAIL  {} - {e}", outcome.target.display());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    println!("
'{name}' is now in your library; deploys will use it unless you say otherwise.");
    Ok(())
}

fn drift(project: Option<PathBuf>) -> Result<()> {
    use inception_core::inventory::DriftStatus;
    let (registry, ctx, scope, settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;
    let groups = inception_core::inventory::skills(&found, scope, settings.library().as_deref());

    let interesting: Vec<_> = groups
        .iter()
        .filter(|g| matches!(g.status, DriftStatus::Drifted | DriftStatus::Problem))
        .collect();

    if interesting.is_empty() {
        println!("No drift found across {} skill(s).", groups.len());
        return Ok(());
    }

    for group in interesting {
        println!("{}  [{}]", group.identity, group.status.label());
        for location in &group.locations {
            match location.entry.skill() {
                Some(skill) => {
                    println!(
                        "    {:<8} {}",
                        &skill.content_hash[..8],
                        location.path.display()
                    );
                    println!("             {}{}", holder(location), changed(location.modified()));
                }
                None => println!("    ????     {}  (unreadable)", location.path.display()),
            }
        }

        // Say *how* they differ, not just that they do.
        //
        // Deliberately not +/-: neither side is "before", and nothing is being
        // changed by looking. The markers name which harness each line came
        // from instead.
        if let Some(comparison) = inception_core::inventory::compare_copies(group)
            && inception_core::diff::has_changes(&comparison.lines)
        {
            println!();
            let side = |s: &inception_core::inventory::CopySide| {
                let who = if s.library { "your library".to_string() } else { s.providers.join(", ") };
                format!("{who}{}", changed(s.modified))
            };
            println!("    < {}  {}", comparison.left.hash, side(&comparison.left));
            println!("    > {}  {}", comparison.right.hash, side(&comparison.right));
            println!();
            for line in &comparison.lines {
                let marker = match line.kind {
                    inception_core::diff::LineKind::Removed => '<',
                    inception_core::diff::LineKind::Added => '>',
                    inception_core::diff::LineKind::Context => ' ',
                };
                println!("    {marker} {}", truncate(&line.text, 96));
            }
        }
        println!();
    }
    Ok(())
}

fn list_mcp(project: Option<PathBuf>) -> Result<()> {
    // No `settings`: MCP servers are not library artifacts yet. Storing one
    // canonically needs a decision about `extra`, which is per-provider by
    // design — see TODO Q-12.
    let (registry, ctx, scope, _settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;
    let servers = inception_core::inventory::mcp_servers(&found, scope);

    if servers.is_empty() {
        println!("No MCP servers found.");
        return Ok(());
    }

    for location in &servers {
        let s = &location.server;
        let where_ = match &location.project {
            Some(p) => format!("{:?} scope, project {p}", location.scope),
            None => format!("{:?} scope", location.scope),
        };
        println!("{}  ({})", s.name, location.provider);
        println!("    transport   {}", s.transport.as_str());
        println!("    {}", where_);
        if s.command.is_empty() {
            println!("    url         {}", s.url.as_deref().unwrap_or("-"));
        } else {
            println!("    command     {}", s.command_line());
        }
        if !s.env.is_empty() {
            // F-7.4: values are never printed.
            let names: Vec<&str> = s.env.iter().map(|(k, _)| k.as_str()).collect();
            println!("    env         {} var(s): {}", names.len(), names.join(", "));
        }
        let secrets = s.secret_like();
        if !secrets.is_empty() {
            println!("    !! plaintext credential(s): {}", secrets.join(", "));
        }
        if !s.extra.is_empty() {
            let keys: Vec<&str> = s.extra.keys().collect();
            println!("    harness-specific (preserved): {}", keys.join(", "));
        }
        println!("    file        {}", location.file.display());
        println!();
    }

    println!("{} MCP server(s) across {} harness(es).", servers.len(), {
        let mut ids: Vec<&str> = servers.iter().map(|l| l.provider.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    });
    Ok(())
}

fn doctor(project: Option<PathBuf>) -> Result<()> {
    use inception_core::doctor::Severity;
    let (registry, ctx, scope, settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;

    let mut findings = inception_core::doctor::check(&found, scope, settings.library().as_deref());
    findings.extend(inception_core::doctor::dangling_references(
        &found,
        scope,
        &inception_core::secrets::Keychain,
    ));

    if findings.is_empty() {
        println!("No problems found.");
        return Ok(());
    }

    for finding in &findings {
        let who = finding
            .provider
            .as_deref()
            .map(|p| format!(" [{p}]"))
            .unwrap_or_default();
        println!("{:<8}{} {}", finding.severity.label(), who, finding.message);
        if let Some(suggestion) = &finding.suggestion {
            println!("         -> {suggestion}");
        }
    }

    let security = findings.iter().filter(|f| f.severity == Severity::Security).count();
    println!("
{} finding(s), {} security-related.", findings.len(), security);
    Ok(())
}

fn deploy(
    name: String,
    to: String,
    from: Option<String>,
    yes: bool,
    project: Option<PathBuf>,
) -> Result<()> {
    use inception_core::plan;
    let (registry, ctx, scope, settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;

    let installed: Vec<String> = found
        .iter()
        .filter(|p| p.installed)
        .map(|p| p.id().to_string())
        .collect();
    let targets: Vec<String> = if to == "all" {
        installed.clone()
    } else {
        to.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    if targets.is_empty() {
        anyhow::bail!("no targets selected");
    }

    // Find the artifact: a skill first, then an MCP server.
    let groups = inception_core::inventory::skills(&found, scope, settings.library().as_deref());
    let plan = if let Some(group) = groups.iter().find(|g| g.identity == name) {
        let chosen = match &from {
            Some(provider) => inception_core::inventory::source_for(group, provider)
                .ok_or_else(|| anyhow::anyhow!("{provider} has no readable copy of '{name}'"))?,
            None => inception_core::inventory::default_source(group)
                .ok_or_else(|| anyhow::anyhow!("'{name}' has no readable copy to deploy from"))?,
        };
        let source = chosen.path.clone();

        // When copies disagree, say which one is about to win. Without --from
        // that is decided by path order, which is no choice at all.
        let versions = inception_core::inventory::versions(group);
        if versions.len() > 1 {
            println!(
                "'{name}' exists in {} versions; using the copy from {}{}",
                versions.len(),
                holder(chosen),
                changed(chosen.modified())
            );
            println!("  {}", source.display());
            if from.is_none() {
                println!("  pass --from <harness> to choose a different one");
            }
            println!();
        }
        plan::skill_deploy(&found, &targets, &source, scope)
    } else {
        let servers = inception_core::inventory::mcp_servers(&found, scope);
        let location = servers
            .iter()
            .find(|l| l.server.name == name)
            .ok_or_else(|| anyhow::anyhow!("no skill or MCP server named '{name}'"))?;
        let targets: Vec<String> =
            targets.into_iter().filter(|t| *t != location.provider).collect();
        plan::mcp_deploy(&found, &targets, &location.server, scope, &inception_core::secrets::Keychain)
    };

    if plan.is_empty() && plan.refusals.is_empty() {
        println!("Nothing to do — '{name}' is already everywhere it was asked to go.");
        return Ok(());
    }

    print!("{}", plan::preview(&plan)?);

    if plan.is_empty() {
        // F-8.2: refusals with nothing to do is a failure, not a success.
        std::process::exit(1);
    }

    println!(
        "{} write(s), serving: {}",
        plan.steps.len(),
        plan.providers_served().join(", ")
    );

    if !yes && !confirm()? {
        println!("Aborted. Nothing was written.");
        return Ok(());
    }

    let mut failed = false;
    for outcome in plan::apply(&plan) {
        match &outcome.result {
            Ok(message) => println!("ok    {} — {message}", outcome.target.display()),
            Err(e) => {
                failed = true;
                eprintln!("FAIL  {} — {e}", outcome.target.display());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn remove(name: String, from: String, yes: bool, project: Option<PathBuf>) -> Result<()> {
    use inception_core::plan;
    let (registry, ctx, scope, settings) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;

    let targets: Vec<String> = if from == "all" {
        found.iter().filter(|p| p.installed).map(|p| p.id().to_string()).collect()
    } else {
        from.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    // Skills first, then MCP servers — matching how `deploy` resolves a name.
    let groups = inception_core::inventory::skills(&found, scope, settings.library().as_deref());
    let plan = if groups.iter().any(|g| g.identity == name) {
        plan::skill_remove(&found, &targets, &name, scope)
    } else {
        plan::mcp_remove(&found, &targets, &name, scope)
    };

    print!("{}", plan::preview(&plan)?);

    if plan.is_empty() {
        for refusal in &plan.refusals {
            eprintln!("nothing to remove: {}", refusal.reason);
        }
        std::process::exit(1);
    }

    let deletes_files = plan.steps.iter().any(|s| matches!(s, plan::Step::SkillRemove(_)));
    if deletes_files {
        println!("Deleting a skill directory cannot be undone from a backup.");
    }

    if !yes && !confirm()? {
        println!("Aborted. Nothing was removed.");
        return Ok(());
    }

    let mut failed = false;
    for outcome in plan::apply(&plan) {
        match &outcome.result {
            Ok(message) => println!("ok    {} - {message}", outcome.target.display()),
            Err(e) => {
                failed = true;
                eprintln!("FAIL  {} - {e}", outcome.target.display());
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// F-5.3: nothing is written without an explicit yes.
fn confirm() -> Result<bool> {
    use std::io::Write;
    print!("
Apply? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

fn status(project: Option<PathBuf>) -> Result<()> {
    let registry = Registry::load();
    report_rejected(&registry);

    let mut ctx = PathContext::detect()?;
    let scope = match &project {
        Some(dir) => {
            ctx = ctx.with_project(std::path::absolute(dir)?);
            Scope::Project
        }
        None => Scope::User,
    };

    let found = inception_core::detect(&registry, &ctx, scope)?;
    let installed: Vec<_> = found.iter().filter(|p| p.installed).collect();

    println!(
        "{} of {} known harnesses detected ({:?} scope)\n",
        installed.len(),
        found.len(),
        scope
    );

    for provider in &found {
        if !provider.installed {
            println!("  {}  {} — not installed", mark(false), provider.spec.name);
            continue;
        }
        println!("  {}  {}", mark(true), provider.spec.name);

        for file in &provider.files {
            // Show every candidate so a fallback layout is visible rather than
            // silently used (F-2.2).
            for (index, candidate) in file.candidates.iter().enumerate() {
                let note = match (index, candidate.state.exists()) {
                    (0, _) => "",
                    (_, true) => "  <- fallback in use",
                    (_, false) => "",
                };
                if candidate.state.exists() || index == 0 {
                    println!(
                        "       config  [{:<7}] {}{}",
                        candidate.state.label(),
                        candidate.path.display(),
                        note
                    );
                }
            }
        }

        for dir in &provider.skill_dirs {
            if !dir.exists {
                continue;
            }
            let mut tags = Vec::new();
            if dir.shared {
                tags.push("shared");
            }
            if dir.canonical {
                tags.push("canonical");
            }
            if dir.deprecated {
                tags.push("DEPRECATED");
            }
            let count = count_skills(&dir.path);
            println!(
                "       skills  [{:<7}] {} ({})",
                format!("{count} found"),
                dir.path.display(),
                tags.join(", ")
            );
        }

        for file in provider.invalid_files() {
            if let inception_core::FileState::Invalid(reason) = &file.state {
                println!("       !! cannot parse {}: {}", file.path.display(), reason);
            }
        }

        for dir in provider.deprecated_in_use() {
            println!(
                "       !! {} is a deprecated location; consider migrating",
                dir.path.display()
            );
        }
    }

    Ok(())
}

/// Rough count for the status view — a proper inventory arrives with `list`.
/// Dot-prefixed directories are harness-managed, not user skills (F-3.2).
fn count_skills(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.path().is_dir()
                && !e.file_name().to_string_lossy().starts_with('.')
                && e.path().join("SKILL.md").is_file()
        })
        .count()
}

fn providers() -> Result<()> {
    let registry = Registry::load();
    report_rejected(&registry);
    print_row("ID", "NAME", "MCP", "SKILLS", "SHARED SKILL DIR");
    for spec in registry.iter() {
        let shared = spec
            .skill_dirs(Scope::User)
            .iter()
            .find(|d| d.shared)
            .map(|d| d.path.clone())
            .unwrap_or_else(|| "-".to_string());
        print_row(
            &spec.id,
            &spec.name,
            yes_no(spec.supports_mcp()),
            yes_no(spec.supports_skills()),
            &shared,
        );
    }
    Ok(())
}

fn validate(path: Option<PathBuf>) -> Result<()> {
    if let Some(path) = path {
        let text = std::fs::read_to_string(&path)?;
        match inception_core::registry::parse_and_validate(&text, &path) {
            Ok(spec) => {
                println!("ok  {} ({})", spec.id, path.display());
                Ok(())
            }
            Err(e) => {
                eprintln!("FAIL  {}", path.display());
                eprintln!("      {e}");
                std::process::exit(1);
            }
        }
    } else {
        let registry = Registry::load();
        for spec in registry.iter() {
            println!("ok  {}", spec.id);
        }
        report_rejected(&registry);
        if registry.rejected.is_empty() {
            println!("\n{} provider spec(s) valid.", registry.len());
            Ok(())
        } else {
            // F-8.2: non-zero exit on error.
            std::process::exit(1);
        }
    }
}

/// F-1.4: a broken spec disables one provider, loudly, without aborting.
fn report_rejected(registry: &Registry) {
    for rejected in &registry.rejected {
        eprintln!("warning: ignoring provider spec {}", rejected.source);
        eprintln!("         {}", rejected.error);
    }
}

/// One helper for header and body so the columns cannot drift apart.
fn print_row(id: &str, name: &str, mcp: &str, skills: &str, shared: &str) {
    println!("{id:<14} {name:<22} {mcp:<6} {skills:<8} {shared}");
}

fn print_skill_row(skill: &str, status: &str, providers: &str) {
    println!("{skill:<26} {status:<11} {providers}");
}

fn mark(on: bool) -> char {
    if on { '+' } else { '-' }
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
