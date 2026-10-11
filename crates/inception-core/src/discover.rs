//! Provider detection (F-2.1, F-2.2, F-2.5).
//!
//! Detection is done purely by probing the paths a spec declares. No harness
//! binary is executed and nothing on disk is evaluated (F-3.5) — a config file is
//! data written by someone else, and we treat it that way.

use crate::error::Result;
use crate::paths::PathContext;
use crate::spec::{Format, ProviderSpec, Scope, SkillDir};
use crate::tree::{self, ConfigFile};
use std::path::PathBuf;

#[derive(Debug)]
pub struct ResolvedFile {
    pub id: String,
    pub scope: Scope,
    pub format: Format,
    /// Every declared candidate, in order, each with its own state. All are kept
    /// rather than just the winner, so `status` can show that a harness is using
    /// a fallback layout (F-2.2).
    pub candidates: Vec<ConfigFile>,
}

impl ResolvedFile {
    /// The candidate a read should use: the first that exists.
    pub fn primary(&self) -> Option<&ConfigFile> {
        self.candidates.iter().find(|c| c.state.exists())
    }

    /// Where a write should go: the first existing candidate, else the first
    /// declared one (PROVIDER-SPEC §3).
    pub fn write_target(&self) -> Option<&ConfigFile> {
        self.primary().or_else(|| self.candidates.first())
    }

    pub fn exists(&self) -> bool {
        self.primary().is_some()
    }
}

#[derive(Debug)]
pub struct ResolvedSkillDir {
    pub path: PathBuf,
    pub scope: Scope,
    pub shared: bool,
    pub canonical: bool,
    pub deprecated: bool,
    pub exists: bool,
    /// Position in this provider's declared list, which is the harness's own
    /// precedence order. Gemini reads `.agents/skills` *before* its own
    /// directory, so "whose version is this" cannot be answered by a
    /// shared-versus-private guess.
    pub order: usize,
}

impl ResolvedSkillDir {
    fn from(spec_dir: &SkillDir, path: PathBuf, order: usize) -> ResolvedSkillDir {
        let exists = path.is_dir();
        ResolvedSkillDir {
            path,
            scope: spec_dir.scope,
            shared: spec_dir.shared,
            canonical: spec_dir.canonical,
            deprecated: spec_dir.deprecated,
            exists,
            order,
        }
    }
}

#[derive(Debug)]
pub struct ResolvedInstructions {
    pub path: PathBuf,
    pub scope: Scope,
    pub shared: bool,
    pub exists: bool,
    /// Where this file sits in the harness's own read order.
    pub order: usize,
}

#[derive(Debug)]
pub struct DetectedProvider<'a> {
    pub spec: &'a ProviderSpec,
    pub home: Option<PathBuf>,
    /// True when anything belonging to this harness was found on disk.
    pub installed: bool,
    pub files: Vec<ResolvedFile>,
    pub skill_dirs: Vec<ResolvedSkillDir>,
    pub instructions: Vec<ResolvedInstructions>,
}

impl DetectedProvider<'_> {
    pub fn id(&self) -> &str {
        &self.spec.id
    }

    pub fn file(&self, id: &str) -> Option<&ResolvedFile> {
        self.files.iter().find(|f| f.id == id)
    }

    /// Existing skill dirs for a scope, in declared order.
    pub fn existing_skill_dirs(&self, scope: Scope) -> impl Iterator<Item = &ResolvedSkillDir> {
        self.skill_dirs
            .iter()
            .filter(move |d| d.scope == scope && d.exists)
    }

    /// Instruction files this harness would actually load, in read order.
    pub fn existing_instructions(&self, scope: Scope) -> impl Iterator<Item = &ResolvedInstructions> {
        self.instructions
            .iter()
            .filter(move |f| f.scope == scope && f.exists)
    }

    /// Deprecated paths that actually hold something — a migration hint (F-2.5).
    pub fn deprecated_in_use(&self) -> impl Iterator<Item = &ResolvedSkillDir> {
        self.skill_dirs.iter().filter(|d| d.deprecated && d.exists)
    }

    /// Config files that exist but could not be parsed. Never written to (NF-1.6).
    pub fn invalid_files(&self) -> impl Iterator<Item = &ConfigFile> {
        self.files
            .iter()
            .flat_map(|f| f.candidates.iter())
            .filter(|c| matches!(c.state, tree::FileState::Invalid(_)))
    }
}

/// Probe every provider in the registry.
///
/// `scope` selects which declared paths are examined; `Scope::Project` requires
/// `ctx.project` to be set.
pub fn detect<'a>(
    registry: &'a crate::Registry,
    ctx: &PathContext,
    scope: Scope,
) -> Result<Vec<DetectedProvider<'a>>> {
    let mut found: Vec<DetectedProvider<'a>> = registry
        .iter()
        .map(|spec| detect_one(spec, ctx, scope))
        .collect::<Result<_>>()?;

    // Whether a harness is *installed* is a fact about the machine, not about
    // the project you are looking at. A fresh repository contains none of a
    // harness's files, but the harness is still there and is still a place you
    // can deploy to — without this, project scope opens on "no harnesses found"
    // and there is nothing the user can do from there.
    if scope != Scope::User {
        for (provider, machine) in found.iter_mut().zip(registry.iter()) {
            if provider.installed {
                continue;
            }
            provider.installed = detect_one(machine, ctx, Scope::User)?.installed;
        }
    }

    Ok(found)
}

fn detect_one<'a>(
    spec: &'a ProviderSpec,
    ctx: &PathContext,
    scope: Scope,
) -> Result<DetectedProvider<'a>> {
    let home = ctx.provider_home(spec)?;

    let mut files = Vec::new();
    for file in &spec.files {
        // Claude Code's `local` MCP scope lives inside its *user* file, so a
        // local-scope query must still consider user-scope files.
        let relevant = file.scope == scope || (scope == Scope::Local && file.scope == Scope::User);
        if !relevant {
            continue;
        }
        let Ok(paths) = ctx.expand_candidates(file.path.as_slice(), spec) else {
            // A path needing {project} outside project scope is not an error,
            // it simply does not apply here.
            continue;
        };
        let candidates = paths
            .into_iter()
            .map(|p| tree::read_file(&p, file.format))
            .collect();
        files.push(ResolvedFile {
            id: file.id.clone(),
            scope: file.scope,
            format: file.format,
            candidates,
        });
    }

    let mut skill_dirs = Vec::new();
    for (order, dir) in spec.skill_dirs(scope).into_iter().enumerate() {
        let Ok(path) = ctx.expand(&dir.path, spec) else {
            continue;
        };
        skill_dirs.push(ResolvedSkillDir::from(dir, path, order));
    }

    let mut instructions = Vec::new();
    for (order, file) in spec.instruction_files(scope).into_iter().enumerate() {
        let Ok(path) = ctx.expand(&file.path, spec) else {
            continue;
        };
        instructions.push(ResolvedInstructions {
            exists: path.is_file(),
            path,
            scope: file.scope,
            shared: file.shared,
            order,
        });
    }
    // Installed = there is evidence belonging to *this* harness. A harness with
    // an empty config file counts; one with nothing of its own does not.
    //
    // Shared directories are deliberately excluded. `~/.agents/skills` is read by
    // four harnesses, so its existence says nothing about whether any particular
    // one is installed — counting it reported Gemini CLI as present on a machine
    // where only Codex had ever run.
    let installed = home.as_ref().is_some_and(|h| h.is_dir())
        || files.iter().any(ResolvedFile::exists)
        || skill_dirs.iter().any(|d| d.exists && !d.shared);

    Ok(DetectedProvider {
        spec,
        home,
        installed,
        files,
        skill_dirs,
        instructions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Registry;

    /// Detection must not blow up when nothing exists — the common case on a
    /// machine with only one or two harnesses installed.
    #[test]
    fn detects_nothing_on_an_empty_home() {
        let ctx = PathContext {
            user_home: PathBuf::from("Z:/definitely/not/here"),
            config: PathBuf::from("Z:/definitely/not/here/.config"),
            project: None,
        };
        let registry = Registry::builtin();
        let found = detect(&registry, &ctx, Scope::User).unwrap();
        assert_eq!(found.len(), 5);
        assert!(found.iter().all(|p| !p.installed));
    }

    /// Project-scope paths must resolve when a project is set, and be skipped
    /// (not error) when it is not.
    #[test]
    fn project_scope_requires_a_project_but_degrades_cleanly() {
        let registry = Registry::builtin();

        let no_project = PathContext {
            user_home: PathBuf::from("Z:/nope"),
            config: PathBuf::from("Z:/nope/.config"),
            project: None,
        };
        let found = detect(&registry, &no_project, Scope::Project).unwrap();
        assert!(found.iter().all(|p| p.files.is_empty() && p.skill_dirs.is_empty()));

        let with_project = no_project.clone().with_project("Z:/nope/repo");
        let found = detect(&registry, &with_project, Scope::Project).unwrap();
        assert!(found.iter().any(|p| !p.skill_dirs.is_empty()));
    }
}
