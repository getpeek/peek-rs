//! Whether a list value survives substitution, from
//! `~/labs/peek/src/canvas/nodes/Variable/listQuoting.ts`.
//!
//! A list variable is inlined into the query as `lines.join(", ")`, so every line lands in the
//! SQL verbatim: anything that is not a short numeric literal has to carry its own quotes or
//! the statement will not parse, and the user only finds out once the query has run.

/// Past this, a run of digits is far more likely an id or code stored as text than a number
/// the database will accept bare.
const MAX_NUMERIC_LENGTH: usize = 8;

pub(super) fn needs_quoting(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('\'') {
        return false;
    }
    !is_numeric(trimmed) || trimmed.len() > MAX_NUMERIC_LENGTH
}

pub(super) fn unquoted(lines: &[String]) -> usize {
    lines.iter().filter(|line| needs_quoting(line)).count()
}

/// Quotes every unquoted line, not only the flagged ones, so the list stays homogeneous — a
/// half-quoted `IN` list is a trap for whoever edits it next.
pub(super) fn quote_all(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| quote(line)).collect()
}

fn quote(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('\'') {
        return line.to_string();
    }
    format!("'{}'", trimmed.replace('\'', "''"))
}

/// `/^-?\d+(\.\d+)?$/`.
fn is_numeric(value: &str) -> bool {
    let value = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (value, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    digits(whole) && fraction.is_none_or(digits)
}

#[cfg(test)]
mod tests {
    use super::{needs_quoting, quote_all, unquoted};

    #[test]
    fn blank_and_already_quoted_lines_are_left_alone() {
        assert!(!needs_quoting("   "));
        assert!(!needs_quoting("'alice'"));
    }

    #[test]
    fn short_numbers_pass_and_long_digit_runs_do_not() {
        assert!(!needs_quoting("42"));
        assert!(!needs_quoting("-3.5"));
        assert!(needs_quoting("123456789"));
        assert!(needs_quoting("alice"));
    }

    #[test]
    fn quoting_escapes_the_quote_itself() {
        let lines = vec!["o'brien".to_string(), "7".to_string(), String::new()];
        assert_eq!(unquoted(&lines), 1);
        assert_eq!(
            quote_all(&lines),
            vec!["'o''brien'".to_string(), "'7'".to_string(), String::new()]
        );
    }
}
