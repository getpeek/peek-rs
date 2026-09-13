//! `categorizeType` from `~/labs/peek/src/canvas/nodes/TableDefinition/columnType.ts`, which
//! sorts a raw SQL type spelling into the eight buckets the table colours by.

/// The eight buckets `columnType.ts` recognises, in the order it tests them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypeCategory {
    Numeric,
    Text,
    Boolean,
    Datetime,
    Json,
    Uuid,
    Binary,
    Other,
}

const NUMERIC: &[&str] = &[
    "int",
    "int2",
    "int4",
    "int8",
    "smallint",
    "bigint",
    "tinyint",
    "mediumint",
    "serial",
    "bigserial",
    "smallserial",
    "float",
    "float4",
    "float8",
    "double",
    "real",
    "decimal",
    "numeric",
    "money",
];
const TEXT: &[&str] = &[
    "text", "varchar", "char", "citext", "name", "bpchar", "string",
];
const BOOLEAN: &[&str] = &["bool", "boolean"];
const DATETIME: &[&str] = &[
    "timestamp",
    "timestamptz",
    "date",
    "time",
    "timetz",
    "interval",
    "datetime",
];
const JSON: &[&str] = &["json", "jsonb"];
const UUID: &[&str] = &["uuid"];
const BINARY: &[&str] = &["bytea", "blob"];

/// A type matches a name exactly or carries a parameter list: `numeric` and `numeric(10,2)`.
fn matches(column_type: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        column_type == *name
            || column_type
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('('))
    })
}

impl TypeCategory {
    pub(super) fn of(raw_type: &str) -> Self {
        let column_type = raw_type.trim().to_lowercase();
        let buckets = [
            (NUMERIC, Self::Numeric),
            (TEXT, Self::Text),
            (BOOLEAN, Self::Boolean),
            (DATETIME, Self::Datetime),
            (JSON, Self::Json),
            (UUID, Self::Uuid),
            (BINARY, Self::Binary),
        ];
        buckets
            .into_iter()
            .find(|(names, _)| matches(&column_type, names))
            .map_or(Self::Other, |(_, category)| category)
    }
}

#[cfg(test)]
mod tests {
    use super::TypeCategory;
    use super::TypeCategory::{Binary, Boolean, Datetime, Json, Numeric, Other, Text, Uuid};

    #[test]
    fn postgres_spellings() {
        assert_eq!(TypeCategory::of("int4"), Numeric);
        assert_eq!(TypeCategory::of("bigint"), Numeric);
        assert_eq!(TypeCategory::of("numeric(10,2)"), Numeric);
        assert_eq!(TypeCategory::of("double precision"), Other);
        assert_eq!(TypeCategory::of("character varying(255)"), Other);
        assert_eq!(TypeCategory::of("varchar(255)"), Text);
        assert_eq!(TypeCategory::of("bpchar"), Text);
        assert_eq!(TypeCategory::of("bool"), Boolean);
        assert_eq!(TypeCategory::of("timestamptz"), Datetime);
        assert_eq!(TypeCategory::of("timestamp with time zone"), Other);
        assert_eq!(TypeCategory::of("jsonb"), Json);
        assert_eq!(TypeCategory::of("uuid"), Uuid);
        assert_eq!(TypeCategory::of("bytea"), Binary);
        assert_eq!(TypeCategory::of("tsvector"), Other);
    }

    #[test]
    fn mysql_spellings() {
        assert_eq!(TypeCategory::of("tinyint(1)"), Numeric);
        assert_eq!(TypeCategory::of("mediumint"), Numeric);
        assert_eq!(TypeCategory::of("decimal(10,2)"), Numeric);
        assert_eq!(TypeCategory::of("char(36)"), Text);
        assert_eq!(TypeCategory::of("longtext"), Other);
        assert_eq!(TypeCategory::of("boolean"), Boolean);
        assert_eq!(TypeCategory::of("datetime"), Datetime);
        assert_eq!(TypeCategory::of("json"), Json);
        assert_eq!(TypeCategory::of("blob"), Binary);
        assert_eq!(TypeCategory::of("enum('a','b')"), Other);
    }

    #[test]
    fn spelling_is_normalised_before_matching() {
        assert_eq!(TypeCategory::of("  UUID  "), Uuid);
        assert_eq!(TypeCategory::of("VARCHAR(80)"), Text);
        assert_eq!(TypeCategory::of(""), Other);
    }

    /// `int` matching `int8` by prefix would make every bucket order-dependent; only a
    /// parameter list may follow a name.
    #[test]
    fn a_longer_spelling_is_not_a_prefix_match() {
        assert_eq!(TypeCategory::of("integer"), Other);
        assert_eq!(TypeCategory::of("serialised"), Other);
        assert_eq!(TypeCategory::of("timestamp_ms"), Other);
    }
}
