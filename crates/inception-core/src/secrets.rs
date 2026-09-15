//! Secret storage (F-7.x).
//!
//! The problem this solves: "deploy this server everywhere" would otherwise copy
//! one API key into five config files in five places on disk. The dev machine
//! already has a plaintext key sitting in `opencode.jsonc`; fanning it out four
//! more times makes a bad situation worse.
//!
//! So a credential is stored **once** in the OS keychain (Windows Credential
//! Manager, macOS Keychain, Secret Service) and configs carry a *reference*,
//! resolved only at the moment of writing.
//!
//! Nothing here ever logs, displays or returns a secret except to the emitter
//! that is about to write it (F-7.4).

use crate::error::Result;
use crate::mcp::Secret;

/// Service name under which all our entries live in the OS keychain.
const SERVICE: &str = "project-inception";

pub trait SecretStore {
    /// Turn a `Secret` into the literal text to write into a config.
    fn resolve(&self, secret: &Secret) -> Result<String>;
    /// Save a value under `name`, replacing any previous one.
    fn store(&self, name: &str, value: &str) -> Result<()>;
    /// Whether a reference currently resolves — used by `doctor`.
    fn exists(&self, name: &str) -> bool;
}

/// The real store.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

impl Keychain {
    fn entry(name: &str) -> std::result::Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(SERVICE, name)
    }
}

impl SecretStore for Keychain {
    fn resolve(&self, secret: &Secret) -> Result<String> {
        match secret {
            Secret::Literal(v) => Ok(v.clone()),
            Secret::Ref(name) => Self::entry(name)
                .and_then(|e| e.get_password())
                .map_err(|e| crate::Error::SecretMissing {
                    name: name.clone(),
                    reason: e.to_string(),
                }),
        }
    }

    fn store(&self, name: &str, value: &str) -> Result<()> {
        Self::entry(name)
            .and_then(|e| e.set_password(value))
            .map_err(|e| crate::Error::SecretStore {
                name: name.to_string(),
                reason: e.to_string(),
            })
    }

    fn exists(&self, name: &str) -> bool {
        Self::entry(name).and_then(|e| e.get_password()).is_ok()
    }
}

/// A store with no keychain behind it: literals pass through, references fail.
///
/// Used in tests, and as the honest fallback on a machine with no working
/// credential store — better to refuse than to silently write a placeholder into
/// a config file.
#[derive(Debug, Default, Clone)]
pub struct LiteralsOnly;

impl SecretStore for LiteralsOnly {
    fn resolve(&self, secret: &Secret) -> Result<String> {
        match secret {
            Secret::Literal(v) => Ok(v.clone()),
            Secret::Ref(name) => Err(crate::Error::SecretMissing {
                name: name.clone(),
                reason: "no keychain is configured in this context".to_string(),
            }),
        }
    }

    fn store(&self, _name: &str, _value: &str) -> Result<()> {
        Err(crate::Error::SecretStore {
            name: String::new(),
            reason: "no keychain is configured in this context".to_string(),
        })
    }

    fn exists(&self, _name: &str) -> bool {
        false
    }
}

/// Stable reference name for a credential: `<server>.<env var>`, lowercased.
///
/// Deterministic so the same credential re-imported later resolves to the same
/// keychain entry instead of accumulating duplicates.
pub fn reference_name(server: &str, key: &str) -> String {
    let sanitise = |s: &str| {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect::<String>()
    };
    format!("{}.{}", sanitise(server), sanitise(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_pass_through() {
        let store = LiteralsOnly;
        assert_eq!(
            store.resolve(&Secret::Literal("abc".into())).unwrap(),
            "abc"
        );
    }

    /// A missing reference must fail loudly. Writing an empty string or the
    /// placeholder text into a real config would be worse than not writing.
    #[test]
    fn unresolvable_reference_is_an_error() {
        let store = LiteralsOnly;
        let err = store.resolve(&Secret::Ref("nope".into())).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn reference_names_are_deterministic_and_safe() {
        assert_eq!(
            reference_name("obsidian", "OBSIDIAN_API_KEY"),
            "obsidian.obsidian_api_key"
        );
        // Characters that would be awkward in a keychain entry are flattened.
        assert_eq!(reference_name("my server", "X-Api/Key"), "my_server.x_api_key");
        // Same inputs, same name — no duplicate entries on re-import.
        assert_eq!(
            reference_name("a", "B"),
            reference_name("a", "B")
        );
    }
}
