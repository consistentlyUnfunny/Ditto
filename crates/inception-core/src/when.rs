//! Human-readable file recency.
//!
//! Used only as a **label**, never as a rule. When two copies of a skill
//! disagree, "this one changed on Tuesday, that one in May" is real information
//! for the person choosing between them — but recency must not pick the winner
//! itself, for three reasons:
//!
//!   - A deploy writes the destination, so the *copy* ends up newer than the
//!     thing it was copied from. A newest-wins rule inverts itself the first
//!     time the tool is used.
//!   - Modification time changes without content changing: a git checkout, a
//!     cloud sync, or restoring one of our own backups all rewrite it. Identity
//!     is content — see [`crate::skills::hash`].
//!   - The situation drift resolution exists for is often "I edited the wrong
//!     copy", and recency would confidently pick the mistake.
//!
//! So: a good label, a bad rule.

use std::time::SystemTime;

/// Render `then` relative to `now`.
///
/// `now` is a parameter rather than read from the clock so the behaviour is
/// testable without freezing time. Granularity is deliberately coarse — the
/// reader is choosing between copies, not auditing a filesystem.
pub fn relative(then: SystemTime, now: SystemTime) -> String {
    // A file stamped in the future is a clock skew or a bad archive, not an
    // error worth refusing over. Say so plainly instead of rendering a negative.
    let Ok(elapsed) = now.duration_since(then) else {
        return "dated in the future".to_string();
    };

    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    // Averaged; this is a label, and "about a month" is the claim being made.
    const MONTH: u64 = 2_629_746;
    const YEAR: u64 = 12 * MONTH;

    let seconds = elapsed.as_secs();
    match seconds {
        s if s < MINUTE => "just now".to_string(),
        s if s < HOUR => plural(s / MINUTE, "minute"),
        s if s < DAY => plural(s / HOUR, "hour"),
        s if s < 2 * DAY => "yesterday".to_string(),
        s if s < WEEK => plural(s / DAY, "day"),
        s if s < MONTH => plural(s / WEEK, "week"),
        s if s < YEAR => plural(s / MONTH, "month"),
        s => plural(s / YEAR, "year"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ago(seconds: u64) -> (SystemTime, SystemTime) {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        (now - Duration::from_secs(seconds), now)
    }

    #[test]
    fn renders_each_band() {
        let cases: &[(u64, &str)] = &[
            (5, "just now"),
            (60, "1 minute ago"),
            (3 * 60, "3 minutes ago"),
            (3600, "1 hour ago"),
            (5 * 3600, "5 hours ago"),
            (25 * 3600, "yesterday"),
            (3 * 86400, "3 days ago"),
            (8 * 86400, "1 week ago"),
            (20 * 86400, "2 weeks ago"),
            (40 * 86400, "1 month ago"),
            (150 * 86400, "4 months ago"),
            (400 * 86400, "1 year ago"),
            (1000 * 86400, "2 years ago"),
        ];
        for (seconds, expected) in cases {
            let (then, now) = ago(*seconds);
            assert_eq!(relative(then, now), *expected, "at {seconds}s");
        }
    }

    /// Clock skew and restored archives both produce future stamps. Never panic,
    /// and never render "18446744073709551615 years ago".
    #[test]
    fn a_future_stamp_is_reported_not_wrapped() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let then = now + Duration::from_secs(86_400);
        assert_eq!(relative(then, now), "dated in the future");
    }
}
