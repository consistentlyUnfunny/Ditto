//! Error types.
//!
//! Rust note: `Result<T>` here is an alias for `std::result::Result<T, Error>`,
//! the same idea as a checked exception except it is a return value. `?` after a
//! call means "if this failed, return the error to my caller" — like an implicit
//! `throw`.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

/// Every way the engine can fail.
///
/// `#[derive(thiserror::Error)]` generates the display text from the `#[error]`
/// attribute on each variant, so these strings are what the user actually sees.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("provider spec {path} is not valid TOML: {source}")]
    SpecParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// A spec parsed but broke a rule from PROVIDER-SPEC.md §9.
    ///
    /// Per F-1.4 this disables one provider, it does not abort the program.
    #[error("provider spec '{provider}' is invalid: {reason}")]
    SpecInvalid { provider: String, reason: String },

    #[error("unsupported schema_version {found} in provider '{provider}' (this build supports {supported})")]
    SpecVersion {
        provider: String,
        found: u32,
        supported: u32,
    },

    #[error("could not determine the {what} directory for this platform")]
    NoSuchDirectory { what: &'static str },

    #[error("unknown placeholder '{{{name}}}' in path '{path}'")]
    UnknownPlaceholder { name: String, path: String },

    #[error("placeholder '{{{name}}}' is not available in this context (path '{path}')")]
    PlaceholderNotAvailable { name: String, path: String },

    #[error("invalid JSON Pointer '{pointer}': {reason}")]
    BadPointer { pointer: String, reason: String },

    /// F-5.4: a target that cannot express something must refuse, with the
    /// alternatives spelled out — a useful message, not a silent downgrade.
    #[error("{provider} does not support {what} (supported: {supported})")]
    Unsupported {
        provider: String,
        what: String,
        supported: String,
    },

    #[error("secret '{name}' could not be read from the keychain: {reason}")]
    SecretMissing { name: String, reason: String },

    #[error("secret '{name}' could not be stored in the keychain: {reason}")]
    SecretStore { name: String, reason: String },

    /// A name the user gave that does not match anything installed.
    #[error("{0}")]
    NotFound(String),

    #[error("{path} is not valid {format}: {reason}")]
    ConfigParse {
        path: PathBuf,
        format: &'static str,
        reason: String,
    },
}

impl Error {
    /// Shorthand so call sites read `.map_err(Error::reading(&path))?`
    /// instead of spelling out the struct every time.
    pub fn reading(path: &std::path::Path) -> impl Fn(std::io::Error) -> Error + '_ {
        move |source| Error::ReadFile {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn invalid(provider: impl Into<String>, reason: impl Into<String>) -> Error {
        Error::SpecInvalid {
            provider: provider.into(),
            reason: reason.into(),
        }
    }
}
