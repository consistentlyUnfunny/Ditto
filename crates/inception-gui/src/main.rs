// A desktop window is the product here, so no console should flash up behind it
// on Windows — but keep it in debug builds, where panics and logs are the point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop shell for project-inception.
//!
//! Every command below is a thin projection over `inception-core`. In
//! particular, the deploy preview calls the *same* plan-and-render path the CLI
//! uses, so the diff shown in the window cannot disagree with what a write
//! actually does — which is the whole reason anyone would trust this thing.

mod dto;

use inception_core::spec::Scope;
use inception_core::{PathContext, Registry};

/// Engine handles are cheap to build and the data underneath changes whenever
/// another tool writes a config, so every command re-reads from disk rather than
/// caching. A full scan of five harnesses is a few dozen small files.
fn survey() -> Result<(Registry, PathContext, Scope), String> {
    let registry = Registry::load();
    let ctx = PathContext::detect().map_err(|e| e.to_string())?;
    Ok((registry, ctx, Scope::User))
}

#[tauri::command]
fn overview() -> Result<dto::Overview, String> {
    let (registry, ctx, scope) = survey()?;
    let found = inception_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Column order is the provider order, filtered to what is actually here.
    let order: Vec<String> = found
        .iter()
        .filter(|p| p.installed)
        .map(|p| p.id().to_string())
        .collect();

    let skills = inception_core::inventory::skills(&found, scope)
        .iter()
        .map(|group| dto::skill_row(group, &order))
        .collect();

    // Servers are grouped by name so the same server on two harnesses reads as
    // one row. No drift comparison yet — see TODO F-4.4.
    let mut servers: Vec<dto::RowView> = Vec::new();
    for location in inception_core::inventory::mcp_servers(&found, scope) {
        let index = order.iter().position(|id| *id == location.provider);
        let row = servers.iter_mut().find(|r| r.name == location.server.name);
        match (row, index) {
            (Some(row), Some(index)) => {
                row.cells[index] = dto::CellView {
                    state: "on",
                    path: Some(location.file.display().to_string()),
                };
            }
            (None, Some(index)) => {
                let mut cells: Vec<dto::CellView> = order
                    .iter()
                    .map(|_| dto::CellView { state: "off", path: None })
                    .collect();
                cells[index] = dto::CellView {
                    state: "on",
                    path: Some(location.file.display().to_string()),
                };
                servers.push(dto::RowView {
                    name: location.server.name.clone(),
                    kind: "server",
                    status: "unique",
                    // The matrix answers "what is where". Transport is a property
                    // of the server, not of its placement, so it belongs in the
                    // detail view rather than as a caption under a column.
                    detail: None,
                    cells,
                    ties: vec![],
                    deprecated: false,
                    deprecated_path: None,
                });
            }
            _ => {}
        }
    }

    Ok(dto::Overview {
        scope: dto::scope_label(scope),
        providers: dto::providers(&found)
            .into_iter()
            .filter(|p| p.installed)
            .collect(),
        skills,
        servers,
    })
}

#[tauri::command]
fn detail(name: String) -> Result<dto::DetailView, String> {
    let (registry, ctx, scope) = survey()?;
    let found = inception_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Skills first, then servers — the same resolution order the deploy path
    // uses. Before this, a server row opened the drawer onto an error.
    if let Some(group) = inception_core::inventory::skills(&found, scope)
        .iter()
        .find(|g| g.identity == name)
    {
        return Ok(dto::detail(group, &found));
    }

    inception_core::inventory::mcp_servers(&found, scope)
        .iter()
        .find(|l| l.server.name == name)
        .map(|l| dto::DetailView::Server(Box::new(dto::server_detail(l))))
        .ok_or_else(|| format!("nothing named '{name}' is installed"))
}

/// Open the OS file manager at a path (T-5.12).
///
/// Arguments are passed as argv, never through a shell, so a path containing
/// quotes or semicolons cannot turn into a command. This launches the file
/// manager only — it never executes anything found in a config (NF-4.3).
#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("{path} no longer exists"));
    }

    let result = if cfg!(target_os = "windows") {
        // `/select,` highlights the entry inside its parent.
        std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(&path).spawn()
    } else {
        let dir = if target.is_dir() { target } else { target.parent().unwrap_or(target) };
        std::process::Command::new("xdg-open").arg(dir).spawn()
    };
    result.map(|_| ()).map_err(|e| format!("could not open the file manager: {e}"))
}

#[tauri::command]
fn doctor() -> Result<Vec<dto::FindingView>, String> {
    let (registry, ctx, scope) = survey()?;
    let found = inception_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;
    let mut items = inception_core::doctor::check(&found, scope);
    items.extend(inception_core::doctor::dangling_references(
        &found,
        scope,
        &inception_core::secrets::Keychain,
    ));
    Ok(dto::findings(items))
}

/// Build a plan without touching disk. The window renders this; nothing is
/// written until [`apply`] is called with the same arguments (F-5.3).
#[tauri::command]
fn plan_deploy(
    name: String,
    targets: Vec<String>,
    source: Option<String>,
) -> Result<dto::PlanView, String> {
    build_plan(&name, &targets, source.as_deref()).map(|p| dto::plan(&p))
}

#[tauri::command]
fn apply(
    name: String,
    targets: Vec<String>,
    source: Option<String>,
) -> Result<Vec<dto::OutcomeView>, String> {
    let plan = build_plan(&name, &targets, source.as_deref())?;
    Ok(inception_core::plan::apply(&plan)
        .into_iter()
        .map(|o| dto::OutcomeView {
            target: o.target.display().to_string(),
            providers: o.providers,
            ok: o.result.is_ok(),
            message: match o.result {
                Ok(m) => m,
                Err(e) => e,
            },
        })
        .collect())
}

/// Shared by preview and apply so the two can never diverge.
fn build_plan(
    name: &str,
    targets: &[String],
    source: Option<&str>,
) -> Result<inception_core::plan::Plan, String> {
    let (registry, ctx, scope) = survey()?;
    let found = inception_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Skills first, then MCP servers — the same resolution order as the CLI.
    let groups = inception_core::inventory::skills(&found, scope);
    if let Some(group) = groups.iter().find(|g| g.identity == name) {
        // An explicit source is the whole point of the resolve flow: the caller
        // decides which version wins, rather than path order deciding for them.
        let source = match source {
            Some(path) => {
                let chosen = std::path::PathBuf::from(path);
                if !group.locations.iter().any(|l| l.path == chosen && l.entry.is_ok()) {
                    return Err(format!("{path} is not a readable copy of '{name}'"));
                }
                chosen
            }
            None => inception_core::inventory::default_source(group)
                .map(|l| l.path.clone())
                .ok_or_else(|| format!("'{name}' has no readable copy to deploy from"))?,
        };
        return Ok(inception_core::plan::skill_deploy(&found, targets, &source, scope));
    }

    let servers = inception_core::inventory::mcp_servers(&found, scope);
    let location = servers
        .iter()
        .find(|l| l.server.name == name)
        .ok_or_else(|| format!("no skill or MCP server named '{name}'"))?;
    let targets: Vec<String> = targets
        .iter()
        .filter(|t| **t != location.provider)
        .cloned()
        .collect();
    Ok(inception_core::plan::mcp_deploy(
        &found,
        &targets,
        &location.server,
        scope,
        &inception_core::secrets::Keychain,
    ))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview,
            detail,
            doctor,
            plan_deploy,
            apply,
            reveal
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the inception window");
}
