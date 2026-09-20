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
    inception_core::inventory::skills(&found, scope)
        .iter()
        .find(|g| g.identity == name)
        .map(dto::detail)
        .ok_or_else(|| format!("no skill named '{name}'"))
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
fn plan_deploy(name: String, targets: Vec<String>) -> Result<dto::PlanView, String> {
    build_plan(&name, &targets).map(|p| dto::plan(&p))
}

#[tauri::command]
fn apply(name: String, targets: Vec<String>) -> Result<Vec<dto::OutcomeView>, String> {
    let plan = build_plan(&name, &targets)?;
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
fn build_plan(name: &str, targets: &[String]) -> Result<inception_core::plan::Plan, String> {
    let (registry, ctx, scope) = survey()?;
    let found = inception_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Skills first, then MCP servers — the same resolution order as the CLI.
    let groups = inception_core::inventory::skills(&found, scope);
    if let Some(group) = groups.iter().find(|g| g.identity == name) {
        let source = group
            .locations
            .iter()
            .find(|l| l.entry.is_ok())
            .map(|l| l.path.clone())
            .ok_or_else(|| format!("'{name}' has no readable copy to deploy from"))?;
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
            apply
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the inception window");
}
