//! The tool's own settings — `~/.inception/config.toml` (Q-7).
//!
//! Called *settings* rather than *config* throughout, because "config" in this
//! codebase means a harness's configuration, which is the thing we translate.
//! This file is ours.
//!
//! ## The library
//!
//! An **imported** skill is copied into `~/.inception/library/<name>/`, and that
//! copy is what a deploy uses when the caller does not name a source. Before the
//! library existed, that default was
//! [`crate::inventory::default_source`]'s "first readable copy in path order" —
//! arbitrary, and the reason the UI has to *show* which copy won.
//!
//! The library is deliberately **a location, not a mode**. Turning it off does
//! not select a second set of rules; it stops one directory from being scanned,
//! after which every ranking falls through to the behaviour that shipped before
//! it. That is why `use_library = false` needs no branch in the planner, in
//! drift detection or in the resolve flow — and why the existing test suite
//! covers that state, since it is the state the suite already runs in.
//!
//! Two consequences worth stating plainly:
//!
//!   - The library is **data, not cache**. F-6.4 still promises the *lockfile*
//!     is disposable, but a skill that was imported and never deployed exists
//!     only here. Deleting `~/.inception/library` loses it.
//!   - Nothing imports on your behalf. A deploy reads the library but never
//!     writes to it; `inception import` is the only way in, so adopting a skill
//!     stays a decision rather than a side effect.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Everything the tool knows about how *it* should behave.
///
/// `deny_unknown_fields` for the same reason provider specs use it: a typo
/// should be an error you can see, not a setting that silently does nothing.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    /// Whether `~/.inception/library` participates as a skill location.
    pub use_library: bool,
    /// Projects the user added by hand.
    ///
    /// Discovery reads the projects a *harness* recorded, which is the better
    /// list for somewhere you already work — but it cannot know about a repo
    /// you have not yet opened an agent in. Those live here, because there is
    /// nowhere else honest to put them: writing into a harness's own project
    /// table so that it remembers a directory would be editing another tool's
    /// records to store ours.
    pub projects: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            use_library: true,
            projects: Vec::new(),
        }
    }
}

impl Settings {
    /// Load from `~/.inception/config.toml`, falling back to defaults.
    ///
    /// A missing file is the normal case and means defaults. A *malformed* file
    /// is an error: silently reverting to defaults would turn a typo into the
    /// tool quietly doing something other than what the file asks (NF-1.6
    /// applied to our own data).
    pub fn load() -> Result<Settings> {
        match settings_path() {
            Some(path) if path.is_file() => Settings::read(&path),
            _ => Ok(Settings::default()),
        }
    }

    pub fn read(path: &Path) -> Result<Settings> {
        let text = std::fs::read_to_string(path).map_err(Error::reading(path))?;
        toml::from_str(&text).map_err(|e| Error::ConfigParse {
            path: path.to_path_buf(),
            format: "TOML",
            reason: e.message().to_string(),
        })
    }

    /// Write the settings back to `~/.inception/config.toml`.
    ///
    /// Goes through the same atomic write the rest of the tool uses, so a crash
    /// mid-save cannot leave a half-written settings file that then refuses to
    /// parse on the next run — which would lock the user out of their own tool
    /// until they went and deleted it by hand.
    pub fn save(&self) -> Result<PathBuf> {
        let path = settings_path().ok_or(Error::NoSuchDirectory { what: "home" })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::reading(parent))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::ConfigParse {
            path: path.clone(),
            format: "TOML",
            reason: e.to_string(),
        })?;
        let header = "# project-inception settings.\n# Delete a line to return it to its default.\n\n";
        crate::write::atomic_write(&path, &format!("{header}{text}"))?;
        Ok(path)
    }

    /// Add a project path, ignoring one that is already recorded.
    ///
    /// Compared with [`crate::projects::same`] rather than by string, so one
    /// repo cannot be added twice under two spellings — Windows hands out both
    /// `D:/Projects/x` and `d:\projects\x` for the same directory (T-6.2).
    pub fn add_project(&mut self, path: &Path) -> bool {
        if self.projects.iter().any(|p| crate::projects::same(p, path)) {
            return false;
        }
        self.projects.push(path.to_path_buf());
        true
    }

    /// Forget a hand-added project.
    ///
    /// Returns false when it was not ours to forget. A harness-recorded project
    /// belongs to that harness's config; removing it from our list would make
    /// it vanish from the picker and then reappear on the next scan, which is
    /// worse than saying plainly that it is not ours to remove.
    pub fn forget_project(&mut self, path: &Path) -> bool {
        let before = self.projects.len();
        self.projects.retain(|p| !crate::projects::same(p, path));
        self.projects.len() != before
    }

    /// The library directory, or `None` when the library is switched off.
    ///
    /// Callers pass this straight into [`crate::inventory::skills`], so "off"
    /// and "no home directory" collapse into the same `None` and neither needs
    /// a special case downstream.
    pub fn library(&self) -> Option<PathBuf> {
        if !self.use_library {
            return None;
        }
        library_dir()
    }

    /// The library directory, created if it is not there yet.
    ///
    /// Nothing creates `~/.inception/library` on its own: an import writes
    /// into it, and until the first one there is a path the window can name
    /// but not open. That is how "Show in folder" came to report that the
    /// library "no longer exists" on a fresh install, which is true and
    /// useless — it never existed.
    ///
    /// This creates the empty directory and nothing in it, so the promise
    /// that only an import puts a skill in the library still holds.
    ///
    /// `Ok(None)` when the library is switched off, which is not a failure:
    /// there is simply nothing to create.
    pub fn ensure_library(&self) -> Result<Option<PathBuf>> {
        let Some(dir) = self.library() else {
            return Ok(None);
        };
        std::fs::create_dir_all(&dir).map_err(Error::reading(&dir))?;
        Ok(Some(dir))
    }
}

/// `~/.inception/library`, regardless of whether the library is enabled.
pub fn library_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".inception").join("library"))
}

/// `~/.inception/config.toml`.
pub fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".inception").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_is_on_unless_switched_off() {
        assert!(Settings::default().use_library);
        let off: Settings = toml::from_str("use_library = false").unwrap();
        assert!(!off.use_library);
        assert!(off.library().is_none());
    }

    /// An empty file means defaults, not an error — `#[serde(default)]` on the
    /// struct is what makes a partially-written file legal.
    #[test]
    fn an_empty_file_is_defaults() {
        let s: Settings = toml::from_str("").unwrap();
        assert!(s.use_library);
        assert!(s.projects.is_empty());
    }

    /// A typo must not read as "leave it on". This is the whole reason for
    /// `deny_unknown_fields`.
    #[test]
    fn a_misspelled_key_is_refused() {
        let err = toml::from_str::<Settings>("use_libary = false").unwrap_err();
        assert!(err.message().contains("use_libary"), "got: {err}");
    }

    #[test]
    fn a_malformed_file_reports_the_path() {
        let dir = std::env::temp_dir().join("inception-settings-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "use_library = yes please").unwrap();

        let err = Settings::read(&path).unwrap_err();
        assert!(err.to_string().contains("config.toml"), "got: {err}");
        let _ = std::fs::remove_file(&path);
    }

    /// Windows spells the same directory several ways, and a picker showing one
    /// repo twice is the bug T-6.2 exists to prevent — one scope down.
    #[test]
    fn a_project_is_added_once_however_it_is_spelled() {
        let mut s = Settings::default();
        assert!(s.add_project(Path::new("D:/Projects/thing")));
        assert!(!s.add_project(Path::new(r"d:\projects\thing")));
        assert_eq!(s.projects.len(), 1);

        assert!(s.forget_project(Path::new("D:/PROJECTS/THING")));
        assert!(s.projects.is_empty());
        // Forgetting something we never held is a no-op, not an error.
        assert!(!s.forget_project(Path::new("D:/Projects/thing")));
    }

    /// Switched off, there is nothing to create — and in particular we must not
    /// create the directory anyway, which would leave the tool making a folder
    /// the user has just said they do not want.
    #[test]
    fn a_switched_off_library_has_no_directory_to_make() {
        let off = Settings {
            use_library: false,
            ..Settings::default()
        };
        assert!(off.ensure_library().unwrap().is_none());
    }

    /// A toggle in the window must survive the next scan, which means it has to
    /// survive a save and a load.
    #[test]
    fn settings_round_trip_through_toml() {
        let mut s = Settings {
            use_library: false,
            ..Settings::default()
        };
        s.add_project(Path::new("D:/Projects/thing"));

        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert!(!back.use_library);
        assert_eq!(back.projects.len(), 1);
    }
}
