//! Which columns of a result are keys, and what they point at.
//!
//! Ported from `columnRoles.ts` and `findReferences.ts`. Two sources, and the order matters:
//! the **schema** is authoritative, and the name-shape heuristics (`^id$`, `_id$`) are only a
//! tie-breaker for the common case where a query's columns cannot be traced back to a table —
//! an aggregate, a join, a `SELECT` over a CTE.

use peek_lsp::SchemaIndex;

/// A column somewhere else in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reference {
    pub(super) table: String,
    pub(super) column: String,
}

/// What a column is, as far as the table can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Role {
    /// Something points at this column — or it is called `id`.
    PrimaryKey,
    /// This column points somewhere — or it is called `something_id`.
    ForeignKey,
    #[default]
    Plain,
}

/// A column's role and its references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ColumnRole {
    pub(super) role: Role,
    /// Columns pointing **at** this one.
    pub(super) inbound: Vec<Reference>,
    /// Columns this one points at.
    pub(super) outbound: Vec<Reference>,
}

/// Classifies every column of a result.
///
/// `tables` are the tables the query reads. References are only looked up for a `SELECT`: the
/// reference bails on anything else, because the columns of an `INSERT … RETURNING` are not the
/// table's own in any useful sense.
pub(super) fn classify(
    columns: &[String],
    tables: &[String],
    schema: Option<&SchemaIndex>,
) -> Vec<ColumnRole> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let (inbound, outbound) = schema
                .map(|schema| references(schema, tables, column))
                .unwrap_or_default();
            ColumnRole {
                role: role_of(column, index, &inbound, &outbound),
                inbound,
                outbound,
            }
        })
        .collect()
}

fn references(
    schema: &SchemaIndex,
    tables: &[String],
    column: &str,
) -> (Vec<Reference>, Vec<Reference>) {
    let mut inbound = Vec::new();
    let mut outbound = Vec::new();
    for table in tables {
        let key = (table.clone(), column.to_string());
        if let Some(sources) = schema.fk_incoming.get(&key) {
            inbound.extend(sources.iter().map(|(table, column)| Reference {
                table: table.clone(),
                column: column.clone(),
            }));
        }
        if let Some(targets) = schema.fk_outgoing.get(&key) {
            outbound.extend(targets.iter().map(|(table, column)| Reference {
                table: table.clone(),
                column: column.clone(),
            }));
        }
    }
    (inbound, outbound)
}

/// `classifyColumn`: a real reference wins, then the name shape.
///
/// The first-column special case is the reference's own: a result whose leading column is named
/// `something_id` is almost always keyed by it, even though the name says foreign key.
fn role_of(column: &str, index: usize, inbound: &[Reference], outbound: &[Reference]) -> Role {
    let looks_like_key = column.eq_ignore_ascii_case("id");
    let looks_like_reference = {
        let lower = column.to_ascii_lowercase();
        lower.len() > 3 && lower.ends_with("_id")
    };

    if !inbound.is_empty() || looks_like_key || (index == 0 && looks_like_reference) {
        return Role::PrimaryKey;
    }
    if !outbound.is_empty() || looks_like_reference {
        return Role::ForeignKey;
    }
    Role::Plain
}

#[cfg(test)]
mod tests {
    use peek_lsp::SchemaIndex;
    use std::collections::HashMap;

    use super::{Role, classify};

    fn schema() -> SchemaIndex {
        let mut tables = HashMap::new();
        tables.insert(
            "users".to_string(),
            vec![("id".to_string(), "uuid".to_string())],
        );
        tables.insert(
            "orders".to_string(),
            vec![("user_id".to_string(), "uuid".to_string())],
        );
        let mut references = HashMap::new();
        references.insert("users.id".to_string(), vec!["orders.user_id".to_string()]);
        SchemaIndex::from_raw(tables, references, HashMap::new())
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_column_pointed_at_is_a_primary_key_and_knows_who_points_at_it() {
        let roles = classify(&names(&["id"]), &names(&["users"]), Some(&schema()));
        assert_eq!(roles[0].role, Role::PrimaryKey);
        assert_eq!(roles[0].inbound.len(), 1);
        assert_eq!(roles[0].inbound[0].table, "orders");
        assert_eq!(roles[0].inbound[0].column, "user_id");
    }

    #[test]
    fn a_column_that_points_somewhere_is_a_foreign_key() {
        let roles = classify(
            &names(&["total", "user_id"]),
            &names(&["orders"]),
            Some(&schema()),
        );
        assert_eq!(roles[1].role, Role::ForeignKey);
        assert_eq!(roles[1].outbound[0].table, "users");
        assert_eq!(roles[1].outbound[0].column, "id");
    }

    /// Faithful to `classifyColumn`: the leading-column rule is checked **before** outbound
    /// references, so a result keyed by `user_id` reads as a key even though it also points at
    /// one. Surprising written down, right on screen — that column *is* the row's identity.
    #[test]
    fn the_leading_column_rule_beats_an_outbound_reference() {
        let roles = classify(&names(&["user_id"]), &names(&["orders"]), Some(&schema()));
        assert_eq!(roles[0].role, Role::PrimaryKey);
        assert_eq!(
            roles[0].outbound.len(),
            1,
            "it still knows what it points at, for the chip"
        );
    }

    /// Without a schema — an aggregate, a join, a CTE — the name shape is all there is.
    #[test]
    fn name_shapes_classify_when_there_is_no_schema() {
        let roles = classify(&names(&["id", "user_id", "name"]), &[], None);
        assert_eq!(roles[0].role, Role::PrimaryKey);
        assert_eq!(roles[1].role, Role::ForeignKey);
        assert_eq!(roles[2].role, Role::Plain);
    }

    /// The reference's own special case: a leading `*_id` column is what the result is keyed by.
    #[test]
    fn a_leading_reference_column_reads_as_the_key() {
        let roles = classify(&names(&["user_id", "total"]), &[], None);
        assert_eq!(roles[0].role, Role::PrimaryKey);

        let later = classify(&names(&["total", "user_id"]), &[], None);
        assert_eq!(later[1].role, Role::ForeignKey, "but not elsewhere");
    }

    /// `_id` alone is a column named nothing useful, not a reference.
    #[test]
    fn a_bare_suffix_is_not_a_reference() {
        let roles = classify(&names(&["total", "_id"]), &[], None);
        assert_eq!(roles[1].role, Role::Plain);
    }

    #[test]
    fn classification_ignores_case() {
        let roles = classify(&names(&["ID", "User_Id"]), &[], None);
        assert_eq!(roles[0].role, Role::PrimaryKey);
        assert_eq!(roles[1].role, Role::ForeignKey);
    }

    /// A column the schema says nothing about is plain, whatever else is in the result.
    #[test]
    fn an_unrelated_column_is_plain() {
        let roles = classify(&names(&["email"]), &names(&["users"]), Some(&schema()));
        assert_eq!(roles[0].role, Role::Plain);
        assert!(roles[0].inbound.is_empty());
        assert!(roles[0].outbound.is_empty());
    }
}
