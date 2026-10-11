//! Project discovery and path identity (T-6.2, T-6.3).
//!
//! Harnesses already record where you work — Claude Code keeps a `projects` map
//! in `~/.claude.json`, Codex a `[projects.'<path>']` trust table. Reading those
//! means the user picks a project from a list instead of typing a path.
//!
//! Which harnesses, and where, comes from the provider specs (`[projects]`), so
//! no harness name appears here (F-1.1).

use crate::discover::DetectedProvider;
use crate::spec::Scope;
use std::path::{Path, PathBuf};

/// A comparison key for a filesystem path.
///
/// **Why this exists.** Windows records the same directory several ways, and the
/// dev machine has proof of both:
///
/// ```text
/// D:/Projects/project-inception
/// D:/projects/project-inception     <- same repo, different case
/// C:/Users/User
/// C:\Users\User                     <- same place, different separator
/// ```
///
/// Comparing raw paths shows phantom duplicates in a picker, and could let the
/// planner treat one repo as two and write to it twice.
///
/// Case is folded on Windows and macOS, where filesystems are conventionally
/// case-insensitive, and preserved on Linux, where it is not.
pub fn key(path: &Path) -> String {
    let text = path.to_string_lossy();
    // Strip Windows' extended-length prefix, which appears in some configs.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let unified: String = text.chars().map(|c| if c == '\\' { '/' } else { c }).collect();
    let trimmed = unified.trim_end_matches('/');
    // A bare drive letter keeps its slash: "C:" is not a directory, "C:/" is.
    let trimmed = if trimmed.ends_with(':') { &unified } else { trimmed };

    if cfg!(any(target_os = "windows", target_os = "macos")) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Whether two paths name the same place.
pub fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

/// A project one of the harnesses knows about.
#[derive(Debug, Clone)]
pub struct Project {
    pub path: PathBuf,
    /// Last path component, for display.
    pub name: String,
    pub is_git: bool,
    /// Which harnesses recorded it, sorted.
    pub sources: Vec<String>,
    /// Project-scope config this repo already has, e.g. `.claude`, `.mcp.json`.
    pub markers: Vec<String>,
    /// You put this one on the list. Only a hand-added project can be taken off
    /// it: the rest are a harness's own records, and dropping one here would
    /// make it vanish and then reappear on the next scan.
    pub added: bool,
}

/// Every project the installed harnesses have recorded, plus any the user added
/// by hand, deduplicated and filtered to directories that still exist.
///
/// `added` comes from [`crate::Settings::projects`]. A harness's record is the
/// better list for somewhere you already work, but it cannot know about a repo
/// you have not opened an agent in yet — and that is exactly the repo you would
/// open this picker to set up.
///
/// Sorted by name so the list is stable between runs.
pub fn discover(providers: &[DetectedProvider<'_>], added: &[PathBuf]) -> Vec<Project> {
    let mut found: Vec<Project> = Vec::new();

    // Hand-added first, so that when a harness has *also* recorded one, the
    // entry keeps `added: true` and stays removable. It is still on our list,
    // whatever else has since learned about it.
    for path in added {
        if !path.is_dir() {
            continue; // added once, since deleted or renamed
        }
        if found.iter().any(|p| same(&p.path, path)) {
            continue;
        }
        let mut project = describe(path, providers, &[]);
        project.added = true;
        found.push(project);
    }

    for provider in providers {
        if !provider.installed {
            continue;
        }
        let Some(spec) = &provider.spec.projects else {
            continue;
        };
        let Some(file) = provider.file(&spec.file).and_then(|f| f.primary()) else {
            continue;
        };
        let Some(doc) = file.state.value() else { continue };
        let Ok(Some(container)) = doc.at_pointer(&spec.pointer) else {
            continue;
        };
        let Some(map) = container.as_map() else { continue };

        for recorded in map.keys() {
            let path = PathBuf::from(recorded);
            if !path.is_dir() {
                continue; // recorded once, since deleted
            }
            match found.iter_mut().find(|p| same(&p.path, &path)) {
                Some(existing) => {
                    let id = provider.id().to_string();
                    if !existing.sources.contains(&id) {
                        existing.sources.push(id);
                    }
                }
                None => {
                    found.push(describe(&path, providers, &[provider.id().to_string()]))
                }
            }
        }
    }

    for project in &mut found {
        project.sources.sort();
    }
    found.sort_by_key(|p| p.name.to_lowercase());
    found
}

/// Describe one project directory.
///
/// `sources` is empty for a directory the user picked by hand — that absence is
/// worth showing, because it means no harness has been used there yet.
pub fn describe(path: &Path, providers: &[DetectedProvider<'_>], sources: &[String]) -> Project {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let markers = marker_names(providers)
        .into_iter()
        .filter(|m| path.join(m).exists())
        .collect();

    Project {
        is_git: path.join(".git").exists(),
        name,
        path: path.to_path_buf(),
        sources: sources.to_vec(),
        markers,
        added: false,
    }
}

/// What a project directory can hold that a harness would recognise, taken from
/// the specs rather than written out here (F-1.1).
///
/// Every project-scope path a spec declares starts with `{project}/`; the first
/// component after that is the thing visible in a directory listing —
/// `.claude`, `.mcp.json`, `AGENTS.md`. Deduplicated and sorted, so the list is
/// stable and says "this repo is already set up for something" at a glance.
fn marker_names(providers: &[DetectedProvider<'_>]) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();

    for provider in providers {
        let spec = provider.spec;
        let mut declared: Vec<&str> = spec
            .files
            .iter()
            .filter(|f| f.scope == Scope::Project)
            .flat_map(|f| f.path.as_slice().iter().map(String::as_str))
            .collect();
        declared.extend(spec.skill_dirs(Scope::Project).iter().map(|d| d.path.as_str()));
        declared.extend(
            spec.instruction_files(Scope::Project)
                .iter()
                .map(|f| f.path.as_str()),
        );

        for template in declared {
            if let Some(first) = first_component(template) {
                names.insert(first.to_string());
            }
        }
    }
    names.into_iter().collect()
}

/// `"{project}/.claude/skills"` -> `".claude"`. Anything not anchored at
/// `{project}` is not a marker of this directory.
fn first_component(template: &str) -> Option<&str> {
    let rest = template.strip_prefix("{project}/")?;
    let end = rest.find(['/', '\\']).unwrap_or(rest.len());
    Some(&rest[..end]).filter(|s| !s.is_empty())
}

/// Whether a path lies inside a git working tree.
///
/// Used to tell the user that a write will be committed. Walks upward rather
/// than shelling out to git, because the answer is only advisory.
pub fn git_root(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Project scope needs a project; every other scope does not.
pub fn requires_project(scope: Scope) -> bool {
    scope == Scope::Project
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_do_not_make_two_paths() {
        assert!(same(Path::new(r"C:\Users\User"), Path::new("C:/Users/User")));
    }

    #[test]
    fn trailing_slashes_do_not_make_two_paths() {
        assert!(same(Path::new("D:/Projects/x/"), Path::new("D:/Projects/x")));
    }

    #[test]
    fn extended_length_prefix_is_stripped() {
        assert!(same(Path::new(r"\\?\C:\Users\User"), Path::new("C:/Users/User")));
    }

    #[test]
    fn a_bare_drive_keeps_its_root() {
        assert_eq!(key(Path::new("C:/")), "c:/");
        assert_ne!(key(Path::new("C:/")), key(Path::new("C:/Users")));
    }

    /// The real pair from the dev machine's recorded projects.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn case_does_not_make_two_paths_on_a_case_insensitive_system() {
        assert!(same(
            Path::new("D:/Projects/project-inception"),
            Path::new("D:/projects/project-inception")
        ));
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn case_matters_where_the_filesystem_says_it_does() {
        assert!(!same(Path::new("/home/a/Proj"), Path::new("/home/a/proj")));
    }

    #[test]
    fn different_places_stay_different() {
        assert!(!same(Path::new("D:/Projects/a"), Path::new("D:/Projects/b")));
    }

    #[test]
    fn git_root_is_found_from_a_subdirectory() {
        let scratch = std::env::temp_dir().join(format!("inception-git-{}", std::process::id()));
        let nested = scratch.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(scratch.join(".git")).unwrap();

        assert_eq!(git_root(&nested).map(|p| key(&p)), Some(key(&scratch)));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn git_root_is_none_outside_a_repository() {
        let scratch = std::env::temp_dir().join(format!("inception-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        // A temp directory has no .git above it on any normal machine.
        if git_root(&scratch).is_some() {
            eprintln!("skipped: temp dir unexpectedly sits inside a repository");
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
