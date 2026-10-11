//! JSON Pointer (RFC 6901) with the escaping that Windows paths force on us.
//!
//! **This is the sharpest edge in the project** (TODO T-1.10). Claude Code stores
//! project-scoped MCP servers at `/projects/<absolute path>/mcpServers`, so the
//! project path becomes an *object key*. On Windows that key is `C:/Users/User`,
//! and every `/` inside it must be escaped to `~1` — otherwise the pointer reads
//! as four nested levels instead of one key, and we silently address the wrong
//! node or create garbage structure.
//!
//! RFC 6901 in full:
//!   - the empty string addresses the whole document
//!   - otherwise the pointer must begin with `/`
//!   - tokens are separated by `/`
//!   - inside a token, `~0` means `~` and `~1` means `/`
//!   - `~` followed by anything else is invalid

use crate::error::{Error, Result};

/// Escape one token for embedding in a pointer.
///
/// Order matters: `~` must be replaced *before* `/`, otherwise the `~1` produced
/// by escaping a slash would itself be re-escaped into `~01`.
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Reverse of [`escape_token`].
///
/// Done in one pass rather than two `replace` calls, because replacing `~0`
/// first and `~1` second mangles the input `~01` (which legitimately decodes to
/// `~1`, not to `/`).
pub fn unescape_token(token: &str) -> Result<String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            other => {
                return Err(Error::BadPointer {
                    pointer: token.to_string(),
                    reason: match other {
                        Some(c) => format!("'~{c}' is not a valid escape (expected ~0 or ~1)"),
                        None => "pointer ends with a dangling '~'".to_string(),
                    },
                });
            }
        }
    }
    Ok(out)
}

/// Split a pointer into its decoded tokens.
///
/// An empty pointer yields an empty list, meaning "the document root".
pub fn parse(pointer: &str) -> Result<Vec<String>> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    if !pointer.starts_with('/') {
        return Err(Error::BadPointer {
            pointer: pointer.to_string(),
            reason: "must be empty or start with '/'".to_string(),
        });
    }
    // `split('/')` on "/a/b" yields ["", "a", "b"] — skip the leading empty.
    pointer
        .split('/')
        .skip(1)
        .map(|token| {
            unescape_token(token).map_err(|_| Error::BadPointer {
                pointer: pointer.to_string(),
                reason: format!("token '{token}' contains an invalid '~' escape"),
            })
        })
        .collect()
}

/// Check a pointer is well-formed without building the token list.
///
/// Used on spec load (PROVIDER-SPEC §9). Pointers in specs may still contain
/// `{placeholder}` segments at this stage; braces are ordinary characters to
/// RFC 6901, so validating the template is valid.
pub fn validate(pointer: &str) -> Result<()> {
    parse(pointer).map(|_| ())
}

/// Build a pointer from already-decoded tokens, escaping each one.
pub fn build<I, S>(tokens: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = String::new();
    for token in tokens {
        out.push('/');
        out.push_str(&escape_token(token.as_ref()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pointer_is_root() {
        assert_eq!(parse("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_pointer_without_leading_slash() {
        assert!(parse("mcpServers").is_err());
    }

    #[test]
    fn splits_simple_pointer() {
        assert_eq!(parse("/mcpServers").unwrap(), vec!["mcpServers"]);
        assert_eq!(parse("/a/b/c").unwrap(), vec!["a", "b", "c"]);
    }

    /// The Windows case this module exists for. A real key from ~/.claude.json.
    #[test]
    fn windows_project_path_round_trips() {
        let key = "C:/Users/User";
        let escaped = escape_token(key);
        assert_eq!(escaped, "C:~1Users~1User");

        let pointer = format!("/projects/{escaped}/mcpServers");
        let tokens = parse(&pointer).unwrap();
        assert_eq!(tokens, vec!["projects", "C:/Users/User", "mcpServers"]);
        // Three tokens, not five. Without escaping this would address
        // /projects/C:/Users/User/mcpServers — the wrong node entirely.
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn unescaped_windows_path_addresses_the_wrong_node() {
        // Documents the bug we are preventing, so a regression is visible.
        let wrong = parse("/projects/C:/Users/User/mcpServers").unwrap();
        assert_eq!(wrong.len(), 5);
    }

    #[test]
    fn tilde_escapes_before_slash() {
        // A key containing both. Escaping in the wrong order yields "~01".
        assert_eq!(escape_token("a~b/c"), "a~0b~1c");
        assert_eq!(unescape_token("a~0b~1c").unwrap(), "a~b/c");
    }

    #[test]
    fn literal_tilde_one_survives() {
        // "~1" as literal text must encode to "~01", not decode to "/".
        assert_eq!(escape_token("~1"), "~01");
        assert_eq!(unescape_token("~01").unwrap(), "~1");
    }

    #[test]
    fn rejects_invalid_escape() {
        assert!(unescape_token("~2").is_err());
        assert!(unescape_token("trailing~").is_err());
        assert!(parse("/bad~2token").is_err());
    }

    #[test]
    fn build_escapes_each_token() {
        assert_eq!(
            build(["projects", "C:/Users/User", "mcpServers"]),
            "/projects/C:~1Users~1User/mcpServers"
        );
    }

    #[test]
    fn build_and_parse_are_inverses() {
        let tokens = vec!["projects".to_string(), "D:/Projects/a~b".to_string()];
        assert_eq!(parse(&build(&tokens)).unwrap(), tokens);
    }

    #[test]
    fn placeholder_templates_validate() {
        // Specs carry unsubstituted placeholders; braces are ordinary characters.
        assert!(validate("/projects/{project_path}/mcpServers").is_ok());
    }
}
