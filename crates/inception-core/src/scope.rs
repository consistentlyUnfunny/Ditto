//! Moving artifacts between user scope and a project (F-5.9).
//!
//! A transfer is an ordinary deploy whose source happens to sit in the other
//! scope. It lives here rather than in a frontend so the CLI and the GUI cannot
//! drift apart, and it deliberately reuses [`crate::plan::skill_deploy`] — a
//! separate code path would be a second planner to keep honest.

use crate::error::{Error, Result};
use crate::paths::PathContext;
use crate::plan::Plan;
use crate::registry::Registry;
use crate::spec::Scope;
use std::path::Path;

/// Which way a transfer goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the user's account into the project.
    IntoProject,
    /// From the project out to the user's account.
    IntoAccount,
}

impl Direction {
    fn scopes(self) -> (Scope, Scope) {
        match self {
            Direction::IntoProject => (Scope::User, Scope::Project),
            Direction::IntoAccount => (Scope::Project, Scope::User),
        }
    }
}

/// Plan a transfer of the skill `name`.
///
/// `targets` may be empty, meaning every installed harness that supports skills.
/// A transfer is about which *scope* an artifact lives in; which harnesses see
/// it was already decided at the scope it came from.
pub fn transfer(
    registry: &Registry,
    ctx: &PathContext,
    project: &Path,
    name: &str,
    direction: Direction,
    targets: &[String],
    library: Option<&Path>,
) -> Result<Plan> {
    let ctx = ctx.clone().with_project(project);
    let (from, into) = direction.scopes();

    let reading = crate::detect(registry, &ctx, from)?;
    // Reading at user scope picks up the library, so a transfer into a project
    // sources from it for the same reason a deploy does — without the library
    // needing to be a project-scope row.
    let groups = crate::inventory::skills(&reading, from, library);
    let group = groups.iter().find(|g| g.identity == name).ok_or_else(|| {
        Error::NotFound(format!("'{name}' is not at {} scope", from.as_str()))
    })?;
    let source = crate::inventory::default_source(group)
        .map(|l| l.path.clone())
        .ok_or_else(|| Error::NotFound(format!("'{name}' has no readable copy to transfer")))?;

    let writing = crate::detect(registry, &ctx, into)?;

    let all: Vec<String>;
    let targets = if targets.is_empty() {
        all = writing
            .iter()
            .filter(|p| p.installed && p.spec.supports_skills())
            .map(|p| p.id().to_string())
            .collect();
        &all[..]
    } else {
        targets
    };

    Ok(crate::plan::skill_deploy(&writing, targets, &source, into))
}
