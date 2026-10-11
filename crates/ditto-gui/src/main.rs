// A desktop window is the product here, so no console should flash up behind it
// on Windows — but keep it in debug builds, where panics and logs are the point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop shell for Ditto.
//!
//! Every command below is a thin projection over `ditto-core`. In
//! particular, the deploy preview calls the *same* plan-and-render path the CLI
//! uses, so the diff shown in the window cannot disagree with what a write
//! actually does — which is the whole reason anyone would trust this thing.

mod dto;

use ditto_core::spec::Scope;
use ditto_core::{PathContext, Registry, Settings};
use std::path::PathBuf;
use std::sync::Mutex;

/// Which project the window is pointed at, if any.
///
/// The only mutable state in the app. Everything else is re-read from disk, so
/// this is the single thing a command can change that another command sees.
#[derive(Default)]
struct Active {
    project: Mutex<Option<PathBuf>>,
}

impl Active {
    fn get(&self) -> Option<PathBuf> {
        self.project.lock().expect("scope lock").clone()
    }
}

type State<'a> = tauri::State<'a, Active>;

/// Engine handles are cheap to build and the data underneath changes whenever
/// another tool writes a config, so every command re-reads from disk rather than
/// caching. A full scan of five harnesses is a few dozen small files.
///
/// Scope follows the selection: a chosen project means project scope, and no
/// choice means user scope. There is no third state to get out of sync.
fn survey(state: &Active) -> Result<(Registry, PathContext, Scope, Settings), String> {
    let registry = Registry::load();
    let ctx = PathContext::detect().map_err(|e| e.to_string())?;
    let settings = Settings::load().map_err(|e| e.to_string())?;
    match state.get() {
        Some(project) => Ok((registry, ctx.with_project(project), Scope::Project, settings)),
        None => Ok((registry, ctx, Scope::User, settings)),
    }
}

/// User scope, whatever the window is pointed at. Scope transfer needs to read
/// one scope while writing the other (T-6.5).
fn survey_user() -> Result<(Registry, PathContext, Settings), String> {
    Ok((
        Registry::load(),
        PathContext::detect().map_err(|e| e.to_string())?,
        Settings::load().map_err(|e| e.to_string())?,
    ))
}

#[tauri::command]
fn overview(state: State<'_>) -> Result<dto::Overview, String> {
    let (registry, ctx, scope, settings) = survey(&state)?;
    let found = ditto_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Column order is the provider order, filtered to what is actually here.
    let order: Vec<String> = found
        .iter()
        .filter(|p| p.installed)
        .map(|p| p.id().to_string())
        .collect();

    let skills = ditto_core::inventory::skills(&found, scope, settings.library().as_deref())
        .iter()
        .map(|group| dto::skill_row(group, &order))
        .collect();

    // Servers are grouped by name so the same server on two harnesses reads as
    // one row. No drift comparison yet — see TODO F-4.4.
    let mut servers: Vec<dto::RowView> = Vec::new();
    for location in ditto_core::inventory::mcp_servers(&found, scope) {
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
                    // MCP servers are not library artifacts yet (TODO Q-12).
                    library: dto::CellView { state: "off", path: None },
                });
            }
            _ => {}
        }
    }

    // Described against the *installed* set so the markers listed are ones a
    // harness on this machine would actually read.
    let project = state
        .get()
        .map(|dir| dto::project(&ditto_core::projects::describe(&dir, &found, &[])));

    Ok(dto::Overview {
        scope: dto::scope_label(scope),
        // The column appears whenever the library is on, even while empty —
        // its absence is information too, and a column that comes and goes as
        // you import things would be worse than one that is simply there.
        library: settings.use_library && scope == Scope::User,
        project,
        providers: dto::providers(&found)
            .into_iter()
            .filter(|p| p.installed)
            .collect(),
        skills,
        servers,
    })
}

#[tauri::command]
fn detail(name: String, state: State<'_>) -> Result<dto::DetailView, String> {
    let (registry, ctx, scope, settings) = survey(&state)?;
    let found = ditto_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Skills first, then servers — the same resolution order the deploy path
    // uses. Before this, a server row opened the drawer onto an error.
    if let Some(group) = ditto_core::inventory::skills(&found, scope, settings.library().as_deref())
        .iter()
        .find(|g| g.identity == name)
    {
        return Ok(dto::detail(group, &found, settings.use_library));
    }

    ditto_core::inventory::mcp_servers(&found, scope)
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
fn doctor(state: State<'_>) -> Result<Vec<dto::FindingView>, String> {
    let (registry, ctx, scope, settings) = survey(&state)?;
    let found = ditto_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;
    let mut items = ditto_core::doctor::check(&found, scope, settings.library().as_deref());
    items.extend(ditto_core::doctor::dangling_references(
        &found,
        scope,
        &ditto_core::secrets::Keychain,
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
    state: State<'_>,
) -> Result<dto::PlanView, String> {
    build_plan(&state, &name, &targets, source.as_deref()).map(|p| dto::plan(&p))
}

#[tauri::command]
fn apply(
    name: String,
    targets: Vec<String>,
    source: Option<String>,
    state: State<'_>,
) -> Result<Vec<dto::OutcomeView>, String> {
    let plan = build_plan(&state, &name, &targets, source.as_deref())?;
    Ok(ditto_core::plan::apply(&plan).into_iter().map(dto::outcome).collect())
}

/// Shared by preview and apply so the two can never diverge.
fn build_plan(
    state: &Active,
    name: &str,
    targets: &[String],
    source: Option<&str>,
) -> Result<ditto_core::plan::Plan, String> {
    let (registry, ctx, scope, settings) = survey(state)?;
    let found = ditto_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;

    // Skills first, then MCP servers — the same resolution order as the CLI.
    let groups = ditto_core::inventory::skills(&found, scope, settings.library().as_deref());
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
            None => ditto_core::inventory::default_source(group)
                .map(|l| l.path.clone())
                .ok_or_else(|| format!("'{name}' has no readable copy to deploy from"))?,
        };
        return Ok(ditto_core::plan::skill_deploy(&found, targets, &source, scope));
    }

    let servers = ditto_core::inventory::mcp_servers(&found, scope);
    let location = servers
        .iter()
        .find(|l| l.server.name == name)
        .ok_or_else(|| format!("no skill or MCP server named '{name}'"))?;
    let targets: Vec<String> = targets
        .iter()
        .filter(|t| **t != location.provider)
        .cloned()
        .collect();
    Ok(ditto_core::plan::mcp_deploy(
        &found,
        &targets,
        &location.server,
        scope,
        &ditto_core::secrets::Keychain,
    ))
}

// ---------------------------------------------------------------------------
// Scope (T-6.4, T-6.5)
// ---------------------------------------------------------------------------

/// Every project the installed harnesses already know about.
///
/// Read from harness config, not from a directory crawl: a harness recording a
/// project is evidence the user works there, and it keeps us off the filesystem.
#[tauri::command]
fn projects() -> Result<Vec<dto::ProjectView>, String> {
    let (registry, ctx, settings) = survey_user()?;
    let found = ditto_core::detect(&registry, &ctx, Scope::User).map_err(|e| e.to_string())?;
    Ok(ditto_core::projects::discover(&found, &settings.projects)
        .iter()
        .map(dto::project)
        .collect())
}

/// Open the OS folder picker and return what was chosen.
///
/// Deliberately a command of ours rather than the dialog plugin's own JS
/// binding. Two reasons: the frontend keeps its zero-dependency build (no
/// `@tauri-apps/plugin-dialog` package), and the plugin command is never
/// exposed to the webview, so the window cannot be talked into opening file
/// dialogs by anything that ends up running in it.
///
/// `spawn_blocking` matters here. The blocking picker parks its thread until
/// the user answers, and a Tauri command body must not do that on the main
/// thread — that is the thread the dialog itself needs in order to appear.
#[tauri::command]
async fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a project folder")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    // A cancelled dialog is `None`, which is not an error — the caller simply
    // does nothing with it.
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()))
}

/// Put a directory on the list, or take one off it.
///
/// Both go through `Settings`, so the window and `dit projects --add`
/// write the same file and read the same list.
#[tauri::command]
fn add_project(path: String) -> Result<Vec<dto::ProjectView>, String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a directory"));
    }
    let mut settings = ditto_core::Settings::load().map_err(|e| e.to_string())?;
    if settings.add_project(&dir) {
        settings.save().map_err(|e| e.to_string())?;
    }
    projects()
}

#[tauri::command]
fn forget_project(path: String) -> Result<Vec<dto::ProjectView>, String> {
    let dir = std::path::PathBuf::from(&path);
    let mut settings = ditto_core::Settings::load().map_err(|e| e.to_string())?;
    if !settings.forget_project(&dir) {
        // A harness's record is that harness's business. Removing it here would
        // make the row vanish and then come back on the next scan.
        return Err(
            "that project was recorded by a harness, not added by you — clear it in              that harness's own config"
                .to_string(),
        );
    }
    settings.save().map_err(|e| e.to_string())?;
    projects()
}

/// The library switch, readable and writable from the window.
///
/// It lived only in `~/.ditto/config.toml` when the library shipped, which
/// made it a setting you had to already know about to find.
#[tauri::command]
fn library_setting() -> Result<dto::LibraryView, String> {
    let settings = ditto_core::Settings::load().map_err(|e| e.to_string())?;
    // Opening this panel is the first moment the window offers to show the
    // library in a file manager, so it is the right moment for the directory
    // to start existing. A failure here is not worth refusing the panel over
    // — the switch still works, and `exists` tells the window to leave the
    // button out rather than offer one that cannot work.
    let _ = settings.ensure_library();
    Ok(dto::library_view(&settings))
}

#[tauri::command]
fn set_library(enabled: bool) -> Result<dto::LibraryView, String> {
    let mut settings = ditto_core::Settings::load().map_err(|e| e.to_string())?;
    settings.use_library = enabled;
    settings.save().map_err(|e| e.to_string())?;
    let _ = settings.ensure_library();
    Ok(dto::library_view(&settings))
}

/// Point the window at a project, or back at user scope with `None`.
///
/// Validated here rather than at read time so a typo fails at the picker, where
/// the user can see why, instead of as an empty matrix later.
#[tauri::command]
fn set_project(path: Option<String>, state: State<'_>) -> Result<Option<dto::ProjectView>, String> {
    let chosen = match path {
        None => None,
        Some(path) => {
            let dir = PathBuf::from(&path);
            if !dir.is_dir() {
                return Err(format!("{path} is not a directory"));
            }
            Some(dir)
        }
    };
    let view = match &chosen {
        None => None,
        Some(dir) => {
            let (registry, ctx, _settings) = survey_user()?;
            let found =
                ditto_core::detect(&registry, &ctx, Scope::User).map_err(|e| e.to_string())?;
            Some(dto::project(&ditto_core::projects::describe(dir, &found, &[])))
        }
    };
    *state.project.lock().expect("scope lock") = chosen;
    Ok(view)
}

/// Adopt a skill into `~/.ditto/library` (Q-7).
///
/// Separate from `apply` because it is a different decision, not a different
/// target: a deploy sends a skill to harnesses, an import makes a copy the
/// tool's own. Sharing the planner would have made every deploy an import.
fn import_plan(
    state: &Active,
    name: &str,
    source: Option<&str>,
) -> Result<ditto_core::plan::Plan, String> {
    let (registry, ctx, scope, settings) = survey(state)?;
    let library = settings
        .library()
        .ok_or("the library is switched off in ~/.ditto/config.toml")?;
    let found = ditto_core::detect(&registry, &ctx, scope).map_err(|e| e.to_string())?;
    let groups =
        ditto_core::inventory::skills(&found, scope, settings.library().as_deref());
    let group = groups
        .iter()
        .find(|g| g.identity == name)
        .ok_or_else(|| format!("no skill named '{name}'"))?;

    let chosen = match source {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            group
                .locations
                .iter()
                .find(|l| l.path == path && l.entry.is_ok())
                .ok_or_else(|| format!("{} is not a readable copy of '{name}'", path.display()))?
        }
        None => group
            .locations
            .iter()
            .find(|l| !l.library && l.entry.is_ok())
            .ok_or_else(|| format!("'{name}' has no readable copy to import"))?,
    };

    Ok(ditto_core::plan::skill_import(&chosen.path, &library))
}

#[tauri::command]
fn plan_import(
    name: String,
    source: Option<String>,
    state: State<'_>,
) -> Result<dto::PlanView, String> {
    import_plan(&state, &name, source.as_deref()).map(|p| dto::plan(&p))
}

#[tauri::command]
fn apply_import(
    name: String,
    source: Option<String>,
    state: State<'_>,
) -> Result<Vec<dto::OutcomeView>, String> {
    let plan = import_plan(&state, &name, source.as_deref())?;
    Ok(ditto_core::plan::apply(&plan).into_iter().map(dto::outcome).collect())
}

/// Copy an artifact between user scope and the selected project (T-6.5).
///
/// The planning lives in the engine, so the window and the CLI cannot disagree
/// about what a transfer does (F-9.1).
fn transfer_plan(
    state: &Active,
    name: &str,
    to_project: bool,
    targets: &[String],
) -> Result<ditto_core::plan::Plan, String> {
    let project = state
        .get()
        .ok_or("pick a project first — a transfer needs two scopes")?;
    let (registry, ctx, settings) = survey_user()?;
    let direction = if to_project {
        ditto_core::scope::Direction::IntoProject
    } else {
        ditto_core::scope::Direction::IntoAccount
    };
    ditto_core::scope::transfer(
        &registry,
        &ctx,
        &project,
        name,
        direction,
        targets,
        settings.library().as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn plan_transfer(
    name: String,
    to_project: bool,
    targets: Vec<String>,
    state: State<'_>,
) -> Result<dto::PlanView, String> {
    transfer_plan(&state, &name, to_project, &targets).map(|p| dto::plan(&p))
}

#[tauri::command]
fn apply_transfer(
    name: String,
    to_project: bool,
    targets: Vec<String>,
    state: State<'_>,
) -> Result<Vec<dto::OutcomeView>, String> {
    let plan = transfer_plan(&state, &name, to_project, &targets)?;
    Ok(ditto_core::plan::apply(&plan).into_iter().map(dto::outcome).collect())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Active::default())
        .invoke_handler(tauri::generate_handler![
            overview,
            detail,
            doctor,
            plan_deploy,
            apply,
            reveal,
            projects,
            set_project,
            plan_transfer,
            apply_transfer,
            plan_import,
            apply_import,
            add_project,
            forget_project,
            pick_folder,
            library_setting,
            set_library
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Ditto window");
}
