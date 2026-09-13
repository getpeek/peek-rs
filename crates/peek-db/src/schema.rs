//! The database's shape, as the Result node and the SQL language server need it.

use std::collections::BTreeMap;

/// Introspected schema.
///
/// Two shapes here are load-bearing and must not be "tidied":
///
/// - [`Schema::references`] is **inverted**: it is keyed by the column being *pointed at*
///   (`"users.id"`) and lists the columns pointing at it (`["orders.user_id", …]`). That is what
///   `SchemaIndex::from_raw` in `peek-lsp` consumes, and it skips any key without a dot.
/// - [`Schema::primary_keys`] is ordered by `ordinal_position`. The composite-key
///   `WHERE ("a","b") IN ((1,2))` form lines its tuples up by this order, so sorting the columns
///   any other way silently deletes the wrong rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    /// table -> [(column, type)]
    pub tables: BTreeMap<String, Vec<(String, String)>>,
    /// `referenced_table.referenced_column` -> [`referencing_table.referencing_column`, …]
    pub references: BTreeMap<String, Vec<String>>,
    /// table -> primary key columns, in ordinal order
    pub primary_keys: BTreeMap<String, Vec<String>>,
}

impl Schema {
    #[must_use]
    pub fn primary_key(&self, table: &str) -> &[String] {
        self.primary_keys.get(table).map_or(&[], Vec::as_slice)
    }

    /// Columns that point **at** `table.column`.
    #[must_use]
    pub fn inbound(&self, table: &str, column: &str) -> &[String] {
        self.references
            .get(&format!("{table}.{column}"))
            .map_or(&[], Vec::as_slice)
    }

    /// Columns `table.column` points at, found by scanning the inverted map.
    #[must_use]
    pub fn outbound(&self, table: &str, column: &str) -> Vec<&str> {
        let needle = format!("{table}.{column}");
        self.references
            .iter()
            .filter(|(_, sources)| sources.contains(&needle))
            .map(|(target, _)| target.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Schema;

    fn schema() -> Schema {
        let mut schema = Schema::default();
        schema.references.insert(
            "users.id".to_string(),
            vec!["orders.user_id".to_string(), "posts.author_id".to_string()],
        );
        schema
            .primary_keys
            .insert("users".to_string(), vec!["id".to_string()]);
        schema
    }

    #[test]
    fn inbound_reads_the_inverted_map_directly() {
        assert_eq!(
            schema().inbound("users", "id"),
            ["orders.user_id", "posts.author_id"]
        );
        assert!(schema().inbound("users", "email").is_empty());
    }

    #[test]
    fn outbound_scans_for_the_column_as_a_source() {
        assert_eq!(schema().outbound("orders", "user_id"), ["users.id"]);
        assert!(schema().outbound("users", "id").is_empty());
    }

    #[test]
    fn a_table_without_a_primary_key_reports_none() {
        assert_eq!(schema().primary_key("users"), ["id"]);
        assert!(schema().primary_key("logs").is_empty());
    }
}
