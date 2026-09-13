//! The node's header text, from `nodeHeading` in `QueryNode.tsx`.

/// How many characters of the query the header shows.
const MAX: usize = 60;

/// The query reduced to one line: a leading `--` dropped so a comment reads as a title, every
/// line trimmed and joined with a space, truncated to [`MAX`].
///
/// The reference slices to 60 UTF-16 units; this counts characters, which is the same for the
/// ASCII SQL that motivates it and cannot split a multi-byte character the way a byte slice
/// would.
pub(crate) fn heading(query: &str) -> String {
    let body = query.strip_prefix("--").map_or(query, str::trim_start);
    let mut joined = String::with_capacity(body.len());
    for (index, line) in body.lines().enumerate() {
        if index > 0 {
            joined.push(' ');
        }
        joined.push_str(line.trim());
    }
    joined.chars().take(MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_comment_marker_is_dropped() {
        assert_eq!(heading("-- Active users"), "Active users");
        assert_eq!(heading("--Active users"), "Active users");
    }

    #[test]
    fn only_a_leading_marker_is_dropped() {
        assert_eq!(heading("select 1 -- note"), "select 1 -- note");
    }

    #[test]
    fn lines_are_trimmed_and_joined_with_one_space() {
        assert_eq!(
            heading("select *\n  from users\n  where id = 1"),
            "select * from users where id = 1"
        );
    }

    #[test]
    fn the_heading_is_capped_at_sixty_characters() {
        let heading = heading(&"x".repeat(100));
        assert_eq!(heading.chars().count(), MAX);
    }

    /// A byte slice at 60 would panic here; the reference's UTF-16 slice would not.
    #[test]
    fn a_multibyte_query_is_cut_on_a_character_boundary() {
        let heading = heading(&"é".repeat(100));
        assert_eq!(heading.chars().count(), MAX);
    }

    #[test]
    fn an_empty_query_has_no_heading() {
        assert!(heading("").is_empty());
        assert!(heading("   \n  ").trim().is_empty());
    }
}
