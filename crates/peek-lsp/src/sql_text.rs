//! Whole-statement text operations: variable sites, formatting, and the destructive-write guard.
//!
//! These are the pieces of `~/labs/peek/src/canvas/variables.ts` and
//! `nodes/Query/isUnboundedWrite.ts` that are pure SQL knowledge rather than UI, so they live
//! beside the parser instead of in `peek-ui`.

/// One `@name` reference in a query, as a byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSite {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// Every `@name` in `query`, in order.
///
/// A reference only counts when it is not preceded by a word character, so `users@email.com`
/// is an address rather than a reference to `@email` — the negative lookbehind the TypeScript
/// regex uses, which Rust's regex crate cannot express, written out as a scan.
#[must_use]
pub fn variable_sites(query: &str) -> Vec<VariableSite> {
    let bytes = query.as_bytes();
    let mut sites = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'@' {
            index += 1;
            continue;
        }
        if index > 0 && is_word_byte(bytes[index - 1]) {
            index += 1;
            continue;
        }
        let start_of_name = index + 1;
        if start_of_name >= bytes.len() || !is_name_start(bytes[start_of_name]) {
            index += 1;
            continue;
        }
        let mut end = start_of_name + 1;
        while end < bytes.len() && is_word_byte(bytes[end]) {
            end += 1;
        }
        sites.push(VariableSite {
            name: query[start_of_name..end].to_string(),
            start: index,
            end,
        });
        index = end;
    }
    sites
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

/// The result of resolving a query's `@variable` references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substitution {
    /// The query with every known reference replaced. Unknown references are left **verbatim**,
    /// as `substituteVariables` leaves them: a half-substituted query is more useful to look at
    /// than one with holes in it, and the caller refuses to run it anyway.
    pub resolved: String,
    /// Names with no value, in first-seen order and without repeats.
    pub missing: Vec<String>,
}

/// Replaces every `@name` in `query` with the value `lookup` gives for it.
///
/// The caller supplies `lookup` rather than a map so that the crate does not need to know how a
/// variable's value is spelled; `peek_canvas` joins list values before they arrive here.
pub fn substitute(query: &str, lookup: impl Fn(&str) -> Option<String>) -> Substitution {
    let sites = variable_sites(query);
    let mut resolved = String::with_capacity(query.len());
    let mut missing: Vec<String> = Vec::new();
    let mut cursor = 0;

    for site in sites {
        resolved.push_str(&query[cursor..site.start]);
        if let Some(value) = lookup(&site.name) {
            resolved.push_str(&value);
        } else {
            if !missing.contains(&site.name) {
                missing.push(site.name.clone());
            }
            resolved.push_str(&query[site.start..site.end]);
        }
        cursor = site.end;
    }
    resolved.push_str(&query[cursor..]);
    Substitution { resolved, missing }
}

/// Pretty-print `query`, keeping `@variable` references intact.
///
/// `sql-formatter` in the reference app does not understand Peek's variables and would split
/// `@limit` into `@` and `limit`, so each distinct name is swapped for a `__pkvar_N__`
/// identifier, the result is formatted, and the names are swapped back —
/// `formatPreservingVars` in `variables.ts`.
#[must_use]
pub fn format(query: &str) -> String {
    let options = sqlformat::FormatOptions {
        uppercase: Some(true),
        ..sqlformat::FormatOptions::default()
    };

    let sites = variable_sites(query);
    if sites.is_empty() {
        return sqlformat::format(query, &sqlformat::QueryParams::None, &options);
    }

    let mut placeholders: Vec<(String, String)> = Vec::new();
    for site in &sites {
        if !placeholders.iter().any(|(name, _)| name == &site.name) {
            let placeholder = format!("__pkvar_{}__", placeholders.len());
            placeholders.push((site.name.clone(), placeholder));
        }
    }

    let mut swapped = String::with_capacity(query.len());
    let mut cursor = 0;
    for site in &sites {
        swapped.push_str(&query[cursor..site.start]);
        let placeholder = placeholders
            .iter()
            .find(|(name, _)| name == &site.name)
            .map(|(_, placeholder)| placeholder.as_str())
            .unwrap_or_default();
        swapped.push_str(placeholder);
        cursor = site.end;
    }
    swapped.push_str(&query[cursor..]);

    let mut formatted = sqlformat::format(&swapped, &sqlformat::QueryParams::None, &options);
    for (name, placeholder) in &placeholders {
        formatted = formatted.replace(placeholder, &format!("@{name}"));
    }
    formatted
}

/// Whether any statement in `query` would wipe a whole table: a `TRUNCATE`, or a `DELETE`
/// with no `WHERE`. The caller confirms before running one.
#[must_use]
pub fn is_unbounded_write(query: &str) -> bool {
    stripped(query)
        .split(';')
        .any(|statement| !statement.trim().is_empty() && is_unbounded_statement(statement))
}

fn is_unbounded_statement(statement: &str) -> bool {
    let head = statement.trim().to_lowercase();
    if head.starts_with("truncate") {
        return true;
    }
    head.starts_with("delete") && !has_where_word(&head)
}

fn has_where_word(lowercase: &str) -> bool {
    lowercase.match_indices("where").any(|(at, _)| {
        let before_is_word = at
            .checked_sub(1)
            .is_some_and(|index| is_word_byte(lowercase.as_bytes()[index]));
        let after = at + "where".len();
        let after_is_word = lowercase
            .as_bytes()
            .get(after)
            .copied()
            .is_some_and(is_word_byte);
        !before_is_word && !after_is_word
    })
}

/// Blank out comments and string literals so a `WHERE` that is commented out, or sitting
/// inside a string, cannot make an unbounded statement look bounded.
fn stripped(query: &str) -> String {
    let bytes = query.as_bytes();
    let mut out = String::with_capacity(query.len());
    let mut index = 0;

    while index < bytes.len() {
        let rest = &query[index..];
        if rest.starts_with("--") {
            let end = rest.find('\n').map_or(query.len(), |at| index + at);
            out.push(' ');
            index = end;
        } else if let Some(after_open) = rest.strip_prefix("/*") {
            let end = after_open
                .find("*/")
                .map_or(query.len(), |at| index + at + 4);
            out.push(' ');
            index = end;
        } else if bytes[index] == b'\'' || bytes[index] == b'"' {
            out.push(' ');
            index = end_of_literal(bytes, index);
        } else {
            out.push(query[index..].chars().next().unwrap_or(' '));
            index += query[index..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// The index just past the closing quote, honouring backslash escapes as the reference
/// regexes do. An unterminated literal runs to the end.
fn end_of_literal(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    let mut index = open + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_sites_finds_each_reference() {
        let sites = variable_sites("select * from t where a = @one and b = @two");
        let names: Vec<&str> = sites.iter().map(|site| site.name.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn a_variable_site_spans_the_at_sign_and_the_name() {
        let query = "limit @count";
        let site = &variable_sites(query)[0];
        assert_eq!(&query[site.start..site.end], "@count");
    }

    #[test]
    fn an_email_is_not_a_variable_reference() {
        assert!(variable_sites("select 'users@email.com'").is_empty());
    }

    #[test]
    fn a_quoted_variable_still_counts() {
        let sites = variable_sites("select '@email'");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].name, "email");
    }

    #[test]
    fn a_bare_at_sign_is_not_a_reference() {
        assert!(variable_sites("select @ from t").is_empty());
        assert!(variable_sites("select @1 from t").is_empty());
    }

    #[test]
    fn format_uppercases_keywords() {
        assert!(format("select 1").starts_with("SELECT"));
    }

    #[test]
    fn format_keeps_variables_whole() {
        let formatted = format("select * from users where id = @user_id");
        assert!(
            formatted.contains("@user_id"),
            "variable should survive formatting: {formatted}"
        );
        assert!(!formatted.contains("__pkvar"));
    }

    #[test]
    fn format_keeps_a_repeated_variable_on_every_site() {
        let formatted = format("select @x, @y, @x from t");
        assert_eq!(formatted.matches("@x").count(), 2);
        assert_eq!(formatted.matches("@y").count(), 1);
    }

    #[test]
    fn truncate_is_unbounded() {
        assert!(is_unbounded_write("truncate table users"));
        assert!(is_unbounded_write("TRUNCATE users"));
    }

    #[test]
    fn delete_without_where_is_unbounded() {
        assert!(is_unbounded_write("delete from users"));
    }

    #[test]
    fn delete_with_where_is_bounded() {
        assert!(!is_unbounded_write("delete from users where id = 1"));
    }

    #[test]
    fn select_is_never_unbounded() {
        assert!(!is_unbounded_write("select * from users"));
    }

    #[test]
    fn a_commented_out_where_does_not_bound_a_delete() {
        assert!(is_unbounded_write("delete from users -- where id = 1"));
        assert!(is_unbounded_write("delete from users /* where id = 1 */"));
    }

    #[test]
    fn a_where_inside_a_string_does_not_bound_a_delete() {
        assert!(is_unbounded_write("delete from logs having 'where'"));
    }

    #[test]
    fn one_unbounded_statement_among_several_flags_the_whole_query() {
        assert!(is_unbounded_write("select 1; delete from users; select 2"));
    }

    #[test]
    fn a_word_containing_where_does_not_bound_a_delete() {
        assert!(is_unbounded_write("delete from somewhereelse"));
    }

    #[test]
    fn empty_and_blank_queries_are_bounded() {
        assert!(!is_unbounded_write(""));
        assert!(!is_unbounded_write("   ;  ; "));
    }

    #[test]
    fn substitution_replaces_known_names_and_reports_the_rest() {
        let vars = |name: &str| match name {
            "limit" => Some("10".to_string()),
            _ => None,
        };
        let done = substitute("select * from t limit @limit offset @skip", vars);
        assert_eq!(done.resolved, "select * from t limit 10 offset @skip");
        assert_eq!(done.missing, ["skip"]);
    }

    /// An unknown name is reported once however often it appears, and in the order first seen —
    /// the error message lists it to the user.
    #[test]
    fn missing_names_are_deduplicated_in_order() {
        let done = substitute("@b and @a and @b", |_| None);
        assert_eq!(done.missing, ["b", "a"]);
    }

    /// The lookbehind rule has to survive substitution, not just scanning.
    #[test]
    fn an_email_address_is_not_substituted() {
        let done = substitute("select * from t where e = 'users@email.com'", |_| {
            Some("WRONG".to_string())
        });
        assert_eq!(done.resolved, "select * from t where e = 'users@email.com'");
        assert!(done.missing.is_empty());
    }

    #[test]
    fn a_query_without_references_is_returned_unchanged() {
        let done = substitute("select 1", |_| Some("x".to_string()));
        assert_eq!(done.resolved, "select 1");
        assert!(done.missing.is_empty());
    }

    /// Substituting must not corrupt a query containing multi-byte characters: the sites are
    /// byte offsets and slicing on the wrong boundary would panic.
    #[test]
    fn multibyte_text_survives_substitution() {
        let done = substitute("select 'café' , @n", |_| Some("1".to_string()));
        assert_eq!(done.resolved, "select 'café' , 1");
    }
}
