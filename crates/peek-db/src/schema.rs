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
            .get(format!("{table}.{column}").as_str())
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

    /// The schema as the compact DDL the agent tools hand out — one line per table, primary and
    /// foreign keys inline:
    ///
    /// ```text
    /// orders(id int4 PK, user_id int4 ->users.id, total numeric)
    /// ```
    ///
    /// Far fewer tokens than JSON, and self-contained per table so the model never has to
    /// cross-reference two maps. `tables` narrows the output; `None` is the whole schema.
    /// Ported from `~/labs/peek/src/mcp/formatSchema.ts`.
    #[must_use]
    pub fn to_ddl(&self, tables: Option<&[String]>) -> String {
        let forward = self.forward_foreign_keys();
        let mut names: Vec<&String> = match tables {
            Some(wanted) => wanted
                .iter()
                .filter(|name| self.tables.contains_key(*name))
                .collect(),
            None => self.tables.keys().collect(),
        };
        names.sort_unstable();

        if names.is_empty() {
            return "(no tables in schema)".to_string();
        }

        names
            .into_iter()
            .map(|table| {
                let primary_keys = self.primary_key(table);
                let columns = self.tables[table]
                    .iter()
                    .map(|(column, kind)| {
                        let key = if primary_keys.iter().any(|name| name == column) {
                            " PK"
                        } else {
                            ""
                        };
                        let target = forward
                            .get(format!("{table}.{column}").as_str())
                            .map_or_else(String::new, |target| format!(" ->{target}"));
                        format!("{column} {}{key}{target}", abbreviate_type(kind))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{table}({columns})")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// [`Schema::references`] points backwards; the DDL needs each column to name what it
    /// points **at**, so invert it once per render rather than scanning per column.
    fn forward_foreign_keys(&self) -> BTreeMap<&str, &str> {
        let mut forward = BTreeMap::new();
        for (referenced, referencing) in &self.references {
            for source in referencing {
                forward.insert(source.as_str(), referenced.as_str());
            }
        }
        forward
    }
}

/// Canonical type names cost several tokens each and repeat on every column. Exact-match only,
/// so parametrized types (`numeric(10,2)`, `varchar(255)`) pass through untouched.
fn abbreviate_type(kind: &str) -> &str {
    match kind.to_ascii_lowercase().as_str() {
        "timestamp without time zone" => "timestamp",
        "timestamp with time zone" => "timestamptz",
        "time without time zone" => "time",
        "time with time zone" => "timetz",
        "character varying" => "varchar",
        "double precision" => "float8",
        "integer" => "int4",
        "bigint" => "int8",
        "smallint" => "int2",
        "boolean" => "bool",
        _ => kind,
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

#[cfg(test)]
mod ddl_tests {
    use super::Schema;

    fn schema() -> Schema {
        let mut schema = Schema::default();
        schema.tables.insert(
            "orders".to_string(),
            vec![
                ("id".to_string(), "integer".to_string()),
                ("user_id".to_string(), "integer".to_string()),
                ("total".to_string(), "numeric(10,2)".to_string()),
                (
                    "placed_at".to_string(),
                    "timestamp with time zone".to_string(),
                ),
            ],
        );
        schema.tables.insert(
            "users".to_string(),
            vec![("id".to_string(), "bigint".to_string())],
        );
        schema
            .primary_keys
            .insert("orders".to_string(), vec!["id".to_string()]);
        schema
            .primary_keys
            .insert("users".to_string(), vec!["id".to_string()]);
        schema
            .references
            .insert("users.id".to_string(), vec!["orders.user_id".to_string()]);
        schema
    }

    #[test]
    fn tables_render_one_per_line_with_keys_inline() {
        assert_eq!(
            schema().to_ddl(None),
            "orders(id int4 PK, user_id int4 ->users.id, total numeric(10,2), \
             placed_at timestamptz)\nusers(id int8 PK)"
        );
    }

    /// Exact-match only: abbreviating `numeric(10,2)` would lose the precision the model needs.
    #[test]
    fn only_canonical_type_names_are_abbreviated() {
        let ddl = schema().to_ddl(None);
        assert!(ddl.contains("total numeric(10,2)"));
        assert!(ddl.contains("id int4"));
    }

    #[test]
    fn the_filter_narrows_the_output_and_ignores_unknown_names() {
        let wanted = ["users".to_string(), "nope".to_string()];
        assert_eq!(schema().to_ddl(Some(&wanted)), "users(id int8 PK)");
    }

    #[test]
    fn an_empty_schema_says_so_rather_than_returning_nothing() {
        assert_eq!(Schema::default().to_ddl(None), "(no tables in schema)");
        let wanted = ["nope".to_string()];
        assert_eq!(schema().to_ddl(Some(&wanted)), "(no tables in schema)");
    }
}
