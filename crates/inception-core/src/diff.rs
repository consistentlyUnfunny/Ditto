//! Line diffs, shared by everything that shows a change.
//!
//! One implementation so the CLI, the GUI and the deploy preview can never
//! disagree about what changed — the same reason `build_plan` is shared.

use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl LineKind {
    /// The prefix a text renderer puts in front of the line.
    pub fn sign(self) -> char {
        match self {
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::Context => ' ',
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Added => "add",
            LineKind::Removed => "del",
            LineKind::Context => "ctx",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// Grouped diff with `context` unchanged lines around each change.
///
/// Trailing newlines are stripped so a renderer controls its own line breaks.
pub fn lines(before: &str, after: &str, context: usize) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(before, after);
    let mut out = Vec::new();
    for group in diff.grouped_ops(context) {
        for op in group {
            for change in diff.iter_changes(&op) {
                out.push(DiffLine {
                    kind: match change.tag() {
                        ChangeTag::Delete => LineKind::Removed,
                        ChangeTag::Insert => LineKind::Added,
                        ChangeTag::Equal => LineKind::Context,
                    },
                    text: change.value().trim_end_matches(['\n', '\r']).to_string(),
                });
            }
        }
    }
    out
}

/// Render for a terminal, two spaces of indent and a `+`/`-`/` ` sign.
pub fn render(lines: &[DiffLine]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str("  ");
        out.push(line.kind.sign());
        out.push(' ');
        out.push_str(&line.text);
        out.push('\n');
    }
    out
}

/// Whether a diff contains any real change.
pub fn has_changes(lines: &[DiffLine]) -> bool {
    lines.iter().any(|l| l.kind != LineKind::Context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_added_and_removed_lines() {
        let d = lines("a\nb\n", "a\nc\n", 1);
        assert!(d.iter().any(|l| l.kind == LineKind::Removed && l.text == "b"));
        assert!(d.iter().any(|l| l.kind == LineKind::Added && l.text == "c"));
        assert!(d.iter().any(|l| l.kind == LineKind::Context && l.text == "a"));
    }

    #[test]
    fn identical_input_has_no_changes() {
        assert!(!has_changes(&lines("a\nb\n", "a\nb\n", 3)));
    }

    /// Line endings are not a change — the same normalisation drift detection
    /// uses, so the two never disagree about whether something differs.
    #[test]
    fn strips_carriage_returns_from_line_text() {
        let d = lines("a\r\nb\r\n", "a\r\nc\r\n", 1);
        assert!(d.iter().all(|l| !l.text.contains('\r')));
    }

    #[test]
    fn renders_with_signs() {
        let text = render(&lines("a\n", "b\n", 0));
        assert!(text.contains("  - a"));
        assert!(text.contains("  + b"));
    }
}
