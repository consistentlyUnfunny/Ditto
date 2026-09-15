//! `inception` — command-line interface.
//!
//! A thin shell over `inception-core`. All logic lives in the engine so the GUI
//! (M4) and the CLI stay in lockstep (F-9.1).

use anyhow::Result;
use clap::{Parser, Subcommand};
use inception_core::spec::Scope;
use inception_core::{PathContext, Registry};
use std::path::PathBuf;

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
        /// Apply without asking. The diff is still printed.
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        project: Option<PathBuf>,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status { project } => status(project),
        Command::List { what, project } => match what {
            What::Skills => list_skills(project),
            What::Mcp => list_mcp(project),
        },
        Command::Drift { project } => drift(project),
        Command::Deploy { name, to, yes, project } => deploy(name, to, yes, project),
        Command::Doctor { project } => doctor(project),
        Command::Remove { name, from, yes, project } => remove(name, from, yes, project),
        Command::Providers => providers(),
        Command::Validate { path } => validate(path),
    }
}

/// Shared setup: load specs, resolve scope, probe the machine.
fn survey(project: Option<PathBuf>) -> Result<(Registry, PathContext, Scope)> {
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
    Ok((registry, ctx, scope))
}

fn list_skills(project: Option<PathBuf>) -> Result<()> {
    let (registry, ctx, scope) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;
    let groups = inception_core::inventory::skills(&found, scope);

    if groups.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    print_skill_row("SKILL", "STATUS", "AVAILABLE TO");
    for group in &groups {
        print_skill_row(
            &truncate(&group.identity, 25),
            group.status.label(),
            &group.providers().join(", "),
        );
        for location in &group.locations {
            let note = match &location.entry.state {
                inception_core::SkillState::Ok(_) if location.shared => "  (shared dir)",
                inception_core::SkillState::Ok(_) => "",
                inception_core::SkillState::MissingSkillMd => "  !! no SKILL.md",
                inception_core::SkillState::Invalid(_) => "  !! unreadable",
            };
            let deprecated = if location.deprecated { "  !! deprecated path" } else { "" };
            println!("    {}{}{}", location.path.display(), note, deprecated);
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

fn drift(project: Option<PathBuf>) -> Result<()> {
    use inception_core::inventory::DriftStatus;
    let (registry, ctx, scope) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;
    let groups = inception_core::inventory::skills(&found, scope);

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
                    println!("             providers: {}", location.providers.join(", "));
                    println!("             description: {}", truncate(&skill.description, 70));
                }
                None => println!("    ????     {}  (unreadable)", location.path.display()),
            }
        }
        println!();
    }
    Ok(())
}

fn list_mcp(project: Option<PathBuf>) -> Result<()> {
    let (registry, ctx, scope) = survey(project)?;
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
    let (registry, ctx, scope) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;

    let mut findings = inception_core::doctor::check(&found, scope);
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

fn deploy(name: String, to: String, yes: bool, project: Option<PathBuf>) -> Result<()> {
    use inception_core::plan;
    let (registry, ctx, scope) = survey(project)?;
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
    let groups = inception_core::inventory::skills(&found, scope);
    let plan = if let Some(group) = groups.iter().find(|g| g.identity == name) {
        let source = group
            .locations
            .iter()
            .find(|l| l.entry.is_ok())
            .map(|l| l.path.clone())
            .ok_or_else(|| anyhow::anyhow!("skill '{name}' has no readable copy to deploy from"))?;
        if group.locations.len() > 1 {
            println!("note: '{name}' exists in {} places; deploying from {}
",
                group.locations.len(), source.display());
        }
        // Do not deploy back into the directory we are reading from.
        let targets: Vec<String> = targets
            .into_iter()
            .filter(|t| !group.locations.iter().any(|l| l.providers.contains(t) && l.path == source))
            .collect();
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
    let (registry, ctx, scope) = survey(project)?;
    let found = inception_core::detect(&registry, &ctx, scope)?;

    let targets: Vec<String> = if from == "all" {
        found.iter().filter(|p| p.installed).map(|p| p.id().to_string()).collect()
    } else {
        from.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    // Skills first, then MCP servers — matching how `deploy` resolves a name.
    let groups = inception_core::inventory::skills(&found, scope);
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
