//! Path placeholder expansion (F-1.8, PROVIDER-SPEC §2).
//!
//! Specs always write paths with `/`. This module turns a template such as
//! `{home}/skills` into a real `PathBuf` for the current machine.
//!
//! ## Why `{config}` is XDG-style even on Windows
//!
//! `dirs::config_dir()` returns `%APPDATA%` on Windows, but OpenCode does not use
//! it — on this machine its config really is at `C:\Users\User\.config\opencode`.
//! OpenCode follows the XDG convention on every platform. So `{config}` resolves
//! to `$XDG_CONFIG_HOME` or `~/.config` everywhere, which matches observed
//! reality. If a harness ever needs the Windows-native location we add a separate
//! `{app_data}` placeholder rather than changing this one.

use crate::error::{Error, Result};
use crate::spec::ProviderSpec;
use std::path::{Path, PathBuf};

/// The machine-specific values placeholders expand to.
#[derive(Debug, Clone)]
pub struct PathContext {
    pub user_home: PathBuf,
    pub config: PathBuf,
    /// Only set when operating in project scope.
    pub project: Option<PathBuf>,
}

impl PathContext {
    pub fn detect() -> Result<PathContext> {
        let user_home = dirs::home_dir().ok_or(Error::NoSuchDirectory { what: "home" })?;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| user_home.join(".config"));
        Ok(PathContext {
            user_home,
            config,
            project: None,
        })
    }

    pub fn with_project(mut self, project: impl Into<PathBuf>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Resolve a provider's base directory: the `[home].env` variable if it is
    /// set and non-empty, otherwise `[home].default` (F-2.3).
    ///
    /// Returns `None` for a spec with no `[home]` block, which is legal —
    /// such a spec simply cannot use `{home}`.
    pub fn provider_home(&self, spec: &ProviderSpec) -> Result<Option<PathBuf>> {
        let Some(home) = &spec.home else {
            return Ok(None);
        };
        if let Some(var) = &home.env
            && let Some(value) = std::env::var_os(var)
            && !value.is_empty()
        {
            return Ok(Some(PathBuf::from(value)));
        }
        // `{home}` cannot reference itself, so pass None.
        self.expand_with(&home.default, None).map(Some)
    }

    /// Expand a template in the context of a provider.
    pub fn expand(&self, template: &str, spec: &ProviderSpec) -> Result<PathBuf> {
        let home = self.provider_home(spec)?;
        self.expand_with(template, home.as_deref())
    }

    /// Expand a template with an explicit `{home}` value.
    pub fn expand_with(&self, template: &str, provider_home: Option<&Path>) -> Result<PathBuf> {
        // `~/x` is shorthand for `{user_home}/x`.
        let template = match template.strip_prefix("~/") {
            Some(rest) => format!("{{user_home}}/{rest}"),
            None => template.to_string(),
        };

        let mut out = PathBuf::new();
        for (index, segment) in template.split('/').enumerate() {
            if segment.is_empty() {
                // A leading empty segment means the template began with '/',
                // i.e. an absolute POSIX path. Preserve it.
                if index == 0 {
                    out.push("/");
                }
                continue;
            }
            match placeholder_name(segment) {
                None => out.push(segment),
                Some(name) => {
                    let value = match name {
                        "user_home" => self.user_home.clone(),
                        "config" => self.config.clone(),
                        "home" => provider_home.map(Path::to_path_buf).ok_or_else(|| {
                            Error::PlaceholderNotAvailable {
                                name: name.to_string(),
                                path: template.clone(),
                            }
                        })?,
                        "project" => self.project.clone().ok_or_else(|| {
                            Error::PlaceholderNotAvailable {
                                name: name.to_string(),
                                path: template.clone(),
                            }
                        })?,
                        other => {
                            return Err(Error::UnknownPlaceholder {
                                name: other.to_string(),
                                path: template.clone(),
                            });
                        }
                    };
                    out.push(value);
                }
            }
        }
        Ok(out)
    }

    /// Expand every candidate path of a file spec, in declared order.
    pub fn expand_candidates(
        &self,
        candidates: &[String],
        spec: &ProviderSpec,
    ) -> Result<Vec<PathBuf>> {
        let home = self.provider_home(spec)?;
        candidates
            .iter()
            .map(|c| self.expand_with(c, home.as_deref()))
            .collect()
    }
}

/// `"{home}"` -> `Some("home")`. A segment that merely *contains* braces, such as
/// `a{b}c`, is not a placeholder — placeholders occupy a whole segment.
fn placeholder_name(segment: &str) -> Option<&str> {
    segment
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .filter(|name| !name.is_empty() && !name.contains(['{', '}']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PathContext {
        PathContext {
            user_home: PathBuf::from("C:/Users/User"),
            config: PathBuf::from("C:/Users/User/.config"),
            project: Some(PathBuf::from("D:/Projects/demo")),
        }
    }

    fn expand(template: &str) -> PathBuf {
        ctx().expand_with(template, Some(Path::new("C:/Users/User/.codex"))).unwrap()
    }

    #[test]
    fn expands_home() {
        assert_eq!(expand("{home}/config.toml"), PathBuf::from("C:/Users/User/.codex/config.toml"));
    }

    #[test]
    fn expands_user_home_and_tilde_equivalently() {
        assert_eq!(expand("{user_home}/.agents/skills"), expand("~/.agents/skills"));
    }

    #[test]
    fn expands_config() {
        assert_eq!(
            expand("{config}/opencode/opencode.jsonc"),
            PathBuf::from("C:/Users/User/.config/opencode/opencode.jsonc")
        );
    }

    #[test]
    fn expands_project() {
        assert_eq!(expand("{project}/.mcp.json"), PathBuf::from("D:/Projects/demo/.mcp.json"));
    }

    #[test]
    fn rejects_unknown_placeholder() {
        let err = ctx().expand_with("{nope}/x", None).unwrap_err();
        assert!(matches!(err, Error::UnknownPlaceholder { .. }));
    }

    #[test]
    fn reports_project_placeholder_outside_project_scope() {
        let mut c = ctx();
        c.project = None;
        let err = c.expand_with("{project}/.mcp.json", None).unwrap_err();
        assert!(matches!(err, Error::PlaceholderNotAvailable { .. }));
    }

    #[test]
    fn reports_home_placeholder_without_home_block() {
        let err = ctx().expand_with("{home}/x", None).unwrap_err();
        assert!(matches!(err, Error::PlaceholderNotAvailable { .. }));
    }

    #[test]
    fn literal_braces_mid_segment_are_not_placeholders() {
        // Not a placeholder: must be passed through untouched rather than erroring.
        assert_eq!(expand("{home}/we{ird}"), PathBuf::from("C:/Users/User/.codex/we{ird}"));
    }

    /// NF-3.5: paths with spaces must survive.
    #[test]
    fn handles_spaces() {
        let c = PathContext {
            user_home: PathBuf::from("C:/Users/Some User"),
            config: PathBuf::from("C:/Users/Some User/.config"),
            project: None,
        };
        assert_eq!(
            c.expand_with("~/.agents/skills", None).unwrap(),
            PathBuf::from("C:/Users/Some User/.agents/skills")
        );
    }

    /// Every built-in spec's every path template must expand without error.
    #[test]
    fn all_builtin_spec_paths_expand() {
        let registry = crate::Registry::builtin();
        let c = ctx();
        for spec in registry.iter() {
            for file in &spec.files {
                for candidate in file.path.as_slice() {
                    c.expand(candidate, spec)
                        .unwrap_or_else(|e| panic!("{} file '{}': {e}", spec.id, file.id));
                }
            }
            for dir in spec.skills.iter().flat_map(|s| s.dirs.iter()) {
                c.expand(&dir.path, spec)
                    .unwrap_or_else(|e| panic!("{} skill dir '{}': {e}", spec.id, dir.path));
            }
        }
    }
}
