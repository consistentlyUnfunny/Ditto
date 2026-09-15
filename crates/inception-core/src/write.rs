//! Safe file replacement (NF-2.1, NF-2.2).
//!
//! Two guarantees before anything touches a config the user cares about:
//!
//!   1. **Atomic.** Write a temp file in the *same directory*, then rename over
//!      the target. A crash mid-write leaves the original intact — never a
//!      half-written config that the harness then refuses to load. Same directory
//!      matters: a rename across volumes is a copy, which is not atomic.
//!   2. **Backed up.** The first time we modify a file, its previous contents are
//!      copied to `~/.inception/backups/`. Every write is reversible (MVP #8).

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct SaveReport {
    pub path: PathBuf,
    /// `None` when the file did not exist before, so there was nothing to back up.
    pub backup: Option<PathBuf>,
    pub bytes_written: usize,
    /// False when the content was already identical and nothing was touched.
    pub changed: bool,
}

pub fn backup_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".inception").join("backups"))
}

/// Back up `path`, then atomically replace it with `content`.
///
/// If the file already holds exactly `content`, nothing is written and no backup
/// is taken — a no-op deploy should not churn the backup directory.
pub fn save(path: &Path, content: &str) -> Result<SaveReport> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::ReadFile { path: path.to_path_buf(), source: e }),
    };

    if existing.as_deref() == Some(content) {
        return Ok(SaveReport {
            path: path.to_path_buf(),
            backup: None,
            bytes_written: 0,
            changed: false,
        });
    }

    let backup = match &existing {
        Some(text) => Some(write_backup(path, text)?),
        None => None,
    };

    atomic_write(path, content)?;

    Ok(SaveReport {
        path: path.to_path_buf(),
        backup,
        bytes_written: content.len(),
        changed: true,
    })
}

/// Copy the current contents aside before modifying.
///
/// Named `<unix-seconds>-<flattened path>` so backups sort chronologically and
/// the original location is readable without an index file.
pub fn write_backup(path: &Path, content: &str) -> Result<PathBuf> {
    let dir = backup_dir().ok_or(Error::NoSuchDirectory { what: "home" })?;
    std::fs::create_dir_all(&dir).map_err(Error::reading(&dir))?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let target = dir.join(format!("{stamp}-{}", flatten(path)));

    std::fs::write(&target, content).map_err(Error::reading(&target))?;
    Ok(target)
}

/// Turn an absolute path into a single safe filename:
/// `C:\Users\User\.codex\config.toml` -> `C__Users_User_.codex_config.toml`.
fn flatten(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' => out.push(c),
            _ => out.push('_'),
        }
    }
    // Keep filenames manageable; the tail is the informative part.
    if out.len() > 120 {
        out = out[out.len() - 120..].to_string();
    }
    out
}

/// Write via a temp file in the same directory, then rename over the target.
///
/// `std::fs::rename` replaces an existing destination on both Windows and Unix,
/// so this is a single atomic swap rather than a truncate-and-write.
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(Error::reading(parent))?;

    // Same directory as the target, so the rename stays on one volume.
    // The pid keeps concurrent writers from colliding on the temp name.
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
        std::process::id()
    ));

    std::fs::write(&temp, content).map_err(Error::reading(&temp))?;

    if let Err(e) = std::fs::rename(&temp, path) {
        // Leave nothing behind if the swap fails.
        let _ = std::fs::remove_file(&temp);
        return Err(Error::ReadFile { path: path.to_path_buf(), source: e });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that cleans itself up, so tests never touch the real
    /// home directory.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "inception-test-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_write_creates_and_replaces() {
        let scratch = Scratch::new("atomic");
        let file = scratch.path("config.toml");

        atomic_write(&file, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "first");

        atomic_write(&file, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "second");
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        let scratch = Scratch::new("notemp");
        let file = scratch.path("config.toml");
        atomic_write(&file, "x").unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&scratch.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn atomic_write_creates_missing_parent_directories() {
        let scratch = Scratch::new("mkdir");
        let file = scratch.path("a/b/c/config.toml");
        atomic_write(&file, "x").unwrap();
        assert!(file.is_file());
    }

    /// CRLF content must land on disk byte-for-byte (NF-3.4).
    #[test]
    fn atomic_write_does_not_translate_line_endings() {
        let scratch = Scratch::new("crlf");
        let file = scratch.path("crlf.json");
        let content = "{\r\n  \"a\": 1\r\n}\r\n";
        atomic_write(&file, content).unwrap();

        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, content.as_bytes());
        assert_eq!(bytes.windows(2).filter(|w| w == b"\r\n").count(), 3);
    }

    #[test]
    fn flatten_makes_a_windows_path_into_one_filename() {
        let flat = flatten(Path::new(r"C:\Users\User\.codex\config.toml"));
        assert!(!flat.contains('\\') && !flat.contains(':') && !flat.contains('/'));
        assert!(flat.ends_with("config.toml"));
    }

    #[test]
    fn flatten_caps_very_long_paths() {
        let long = PathBuf::from("C:/").join("x".repeat(400));
        assert!(flatten(&long).len() <= 120);
    }

    #[test]
    fn saving_identical_content_is_a_no_op() {
        let scratch = Scratch::new("noop");
        let file = scratch.path("config.toml");
        std::fs::write(&file, "same").unwrap();

        let report = save(&file, "same").unwrap();
        assert!(!report.changed);
        assert!(report.backup.is_none(), "no-op should not create a backup");
    }
}
