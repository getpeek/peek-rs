//! The SQL highlight query, remapped onto the roles a theme can actually colour.

/// `tree-sitter-sequel`'s captures, adjusted for gpui-component's role table.
///
/// The grammar's capture names predate nvim-treesitter's renaming pass, and four of them have
/// no slot among the roles a theme defines, so `CASE`/`WHEN`, `TEMPORARY` and decimal literals
/// would all render at the plain editor foreground.
///
/// `@field` and `@parameter` are deliberately left alone. They are table and column
/// identifiers, and the Monaco themes this port follows paint those at `editor.foreground` on
/// purpose — white is reserved for the data the user is reading.
///
/// `@keyword.operator` would fall back along the dots to `keyword`, losing the second keyword
/// hue every theme defines as `SyntaxSpec::keyword_control`. It is carried on `constant`,
/// which SQL never emits; `peek-theme`'s `syntax_entries` is the other half of that pairing.
#[must_use]
pub fn sql_highlights() -> String {
    tree_sitter_sequel::HIGHLIGHTS_QUERY
        .replace("@keyword.operator", "@constant")
        .replace("@conditional", "@keyword")
        .replace("@storageclass", "@keyword")
        .replace("@float", "@number")
        .replace(" @spell", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_unsupported_capture_survives() {
        let query = sql_highlights();
        for capture in ["@conditional", "@storageclass", "@float", "@spell"] {
            assert!(
                !query.contains(capture),
                "{capture} should have been remapped"
            );
        }
    }

    #[test]
    fn keyword_operator_is_carried_on_constant() {
        let query = sql_highlights();
        assert!(!query.contains("@keyword.operator"));
        assert!(query.contains("@constant"));
    }

    /// `@type` is a prefix of `@type.qualifier`, so a careless `replace` would corrupt it.
    #[test]
    fn dotted_captures_that_resolve_by_fallback_are_untouched() {
        let query = sql_highlights();
        for capture in ["@type.qualifier", "@type.builtin", "@function.call"] {
            assert!(
                query.contains(capture),
                "{capture} should survive: it falls back to its prefix role"
            );
        }
    }

    #[test]
    fn plain_keyword_captures_remain() {
        assert!(sql_highlights().contains("@keyword"));
    }
}
