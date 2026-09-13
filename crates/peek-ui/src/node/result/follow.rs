//! Following a foreign key to the rows on the other side of it.
//!
//! Ported from `useFollowReferences.ts`. Clicking a reference asks the database for the rows it
//! points at — or the rows pointing at it — and fans them onto the canvas beside the result they
//! came from.
//!
//! Two deliberate fixes to the reference's query, which builds
//! `` `SELECT * FROM ${ref.table} WHERE ${ref.column} = '${value}' LIMIT 300` ``:
//!
//! - identifiers go through [`Engine::quote_identifier`], so a table called `order` or a column
//!   called `select` does not produce a syntax error;
//! - the value goes through the literal formatter rather than being wrapped in quotes by string
//!   interpolation, so a key containing an apostrophe cannot end the literal early.

use peek_db::Engine;
use peek_db::mutation::format_cell_literal;
use peek_document::Cell;

use super::column_roles::{ColumnRole, Reference};

/// How many rows a followed reference brings back, from the reference.
const FOLLOW_LIMIT: usize = 300;

/// The references a cell offers to follow.
///
/// Inbound wins: on a primary key, "what points at this row" is the useful question, and the
/// column rarely has both.
#[must_use]
pub(super) fn targets(role: &ColumnRole) -> &[Reference] {
    if role.inbound.is_empty() {
        &role.outbound
    } else {
        &role.inbound
    }
}

/// The queries that follow `role` from a cell holding `value`.
#[must_use]
pub(super) fn queries(
    role: &ColumnRole,
    value: &Cell,
    sql_type: &str,
    engine: Engine,
) -> Vec<String> {
    // A NULL points at nothing; asking `WHERE col = NULL` returns nothing and is never what the
    // user meant by clicking.
    if value.is_null() {
        return Vec::new();
    }
    let literal = format_cell_literal(value, sql_type, engine);
    targets(role)
        .iter()
        .map(|reference| {
            format!(
                "SELECT * FROM {} WHERE {} = {literal} LIMIT {FOLLOW_LIMIT}",
                engine.quote_identifier(&reference.table),
                engine.quote_identifier(&reference.column),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use peek_db::Engine;
    use peek_document::Cell;

    use super::super::column_roles::{ColumnRole, Reference, Role};
    use super::queries;

    fn reference(table: &str, column: &str) -> Reference {
        Reference {
            table: table.to_string(),
            column: column.to_string(),
        }
    }

    fn role(inbound: Vec<Reference>, outbound: Vec<Reference>) -> ColumnRole {
        ColumnRole {
            role: Role::Plain,
            inbound,
            outbound,
        }
    }

    #[test]
    fn following_a_primary_key_asks_what_points_at_it() {
        let built = queries(
            &role(vec![reference("orders", "user_id")], Vec::new()),
            &Cell::Int(7),
            "INT4",
            Engine::Postgres,
        );
        assert_eq!(
            built,
            [r#"SELECT * FROM "orders" WHERE "user_id" = 7 LIMIT 300"#]
        );
    }

    #[test]
    fn following_a_foreign_key_asks_for_what_it_points_at() {
        let built = queries(
            &role(Vec::new(), vec![reference("users", "id")]),
            &Cell::Int(7),
            "INT4",
            Engine::Postgres,
        );
        assert_eq!(built, [r#"SELECT * FROM "users" WHERE "id" = 7 LIMIT 300"#]);
    }

    /// A column with several inbound references produces one query each, which is why the caller
    /// needs the multi-statement fan-out.
    #[test]
    fn every_reference_becomes_its_own_query() {
        let built = queries(
            &role(
                vec![
                    reference("orders", "user_id"),
                    reference("posts", "author_id"),
                ],
                Vec::new(),
            ),
            &Cell::Int(1),
            "INT4",
            Engine::Postgres,
        );
        assert_eq!(built.len(), 2);
        assert!(built[1].contains(r#""posts""#));
    }

    /// Inbound wins: on a primary key, "what points at this row" is the useful question.
    #[test]
    fn inbound_references_are_preferred_over_outbound() {
        let built = queries(
            &role(
                vec![reference("orders", "user_id")],
                vec![reference("elsewhere", "id")],
            ),
            &Cell::Int(1),
            "INT4",
            Engine::Postgres,
        );
        assert_eq!(built.len(), 1);
        assert!(built[0].contains(r#""orders""#));
    }

    /// The reference interpolates the value into quotes, so an apostrophe ends the literal early
    /// and the rest becomes SQL. This escapes it.
    #[test]
    fn a_value_with_an_apostrophe_cannot_break_out() {
        let built = queries(
            &role(Vec::new(), vec![reference("users", "slug")]),
            &Cell::Text("o'brien".into()),
            "VARCHAR",
            Engine::Postgres,
        );
        assert_eq!(
            built[0],
            r#"SELECT * FROM "users" WHERE "slug" = 'o''brien' LIMIT 300"#
        );
    }

    /// A table named after a keyword needs its quotes, or the statement does not parse.
    #[test]
    fn identifiers_are_quoted_for_the_dialect() {
        let built = queries(
            &role(Vec::new(), vec![reference("order", "select")]),
            &Cell::Int(1),
            "INT4",
            Engine::MySql,
        );
        assert_eq!(
            built[0],
            "SELECT * FROM `order` WHERE `select` = 1 LIMIT 300"
        );
    }

    /// Clicking a NULL is never a question worth asking.
    #[test]
    fn a_null_follows_nothing() {
        let built = queries(
            &role(Vec::new(), vec![reference("users", "id")]),
            &Cell::Null,
            "INT4",
            Engine::Postgres,
        );
        assert!(built.is_empty());
    }

    #[test]
    fn a_column_with_no_references_follows_nothing() {
        let built = queries(
            &role(Vec::new(), Vec::new()),
            &Cell::Int(1),
            "INT4",
            Engine::Postgres,
        );
        assert!(built.is_empty());
    }
}
