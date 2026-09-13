//! Identifiers, kept string-typed so they round-trip byte-for-byte with the TypeScript app.
//! Fresh ids use the same `nanoid(8)` alphabet and the same prefixes as `src/canvas/ids.ts`,
//! so documents authored here are indistinguishable from the Tauri app's.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::kinds::NodeType;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                Self(raw.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(NodeId);
id_type!(PageId);
id_type!(EdgeId);
id_type!(RegionId);

fn fresh(prefix: &str) -> String {
    format!("{prefix}_{}", nanoid::nanoid!(8))
}

impl PageId {
    #[must_use]
    pub fn generate() -> Self {
        Self(fresh("page"))
    }
}

impl RegionId {
    #[must_use]
    pub fn generate() -> Self {
        Self(fresh("region"))
    }
}

impl NodeId {
    #[must_use]
    pub fn query() -> Self {
        Self(fresh("query"))
    }

    #[must_use]
    pub fn agent() -> Self {
        Self(fresh("agent"))
    }

    #[must_use]
    pub fn text() -> Self {
        Self(fresh("text"))
    }

    #[must_use]
    pub fn variable() -> Self {
        Self(fresh("variable"))
    }

    #[must_use]
    pub fn draw() -> Self {
        Self(fresh("draw"))
    }

    #[must_use]
    pub fn activity() -> Self {
        Self(fresh("activity"))
    }

    #[must_use]
    pub fn result_insert_form() -> Self {
        Self(fresh("resultform"))
    }

    /// Result nodes are addressed by construction from their query, not by edge lookup.
    #[must_use]
    pub fn result_of(parent: &NodeId, index: usize) -> Self {
        Self(format!("{parent}-result-{index}"))
    }

    #[must_use]
    pub fn chart_of(parent: &NodeId) -> Self {
        Self(format!("{parent}-chart"))
    }

    #[must_use]
    pub fn error_of(parent: &NodeId) -> Self {
        Self(format!("{parent}-error"))
    }

    /// The id `makeNode` would mint for a freshly created node of this type.
    ///
    /// Derived kinds get an id built from a throwaway query id, exactly as `defaults.ts` does,
    /// and `table-definition` really does take the `query_` prefix: `ids.ts` has no prefix of
    /// its own for it.
    #[must_use]
    pub fn for_type(node_type: NodeType) -> Self {
        match node_type {
            NodeType::Query | NodeType::TableDefinition => Self::query(),
            NodeType::Agent => Self::agent(),
            NodeType::Text => Self::text(),
            NodeType::Variable => Self::variable(),
            NodeType::Draw => Self::draw(),
            NodeType::Activity => Self::activity(),
            NodeType::ResultInsertForm => Self::result_insert_form(),
            NodeType::Result => Self::result_of(&Self::query(), 0),
            NodeType::Barchart => Self::chart_of(&Self::query()),
            NodeType::QueryError => Self::error_of(&Self::query()),
        }
    }
}

impl EdgeId {
    #[must_use]
    pub fn between(source: &NodeId, target: &NodeId) -> Self {
        Self(format!("{source}->{target}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_as_bare_strings() {
        let id = NodeId::from("query_abc12345");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"query_abc12345\"");
        assert_eq!(
            EdgeId::between(&id, &NodeId::result_of(&id, 0)).as_str(),
            "query_abc12345->query_abc12345-result-0"
        );
    }

    #[test]
    fn for_type_matches_the_typescript_prefixes() {
        let cases = [
            (NodeType::Query, "query_"),
            (NodeType::TableDefinition, "query_"),
            (NodeType::Agent, "agent_"),
            (NodeType::Text, "text_"),
            (NodeType::Variable, "variable_"),
            (NodeType::Draw, "draw_"),
            (NodeType::Activity, "activity_"),
            (NodeType::ResultInsertForm, "resultform_"),
        ];
        for (node_type, prefix) in cases {
            let id = NodeId::for_type(node_type);
            assert!(id.as_str().starts_with(prefix), "{node_type:?} -> {id}");
        }
        assert!(
            NodeId::for_type(NodeType::Result)
                .as_str()
                .ends_with("-result-0")
        );
        assert!(
            NodeId::for_type(NodeType::Barchart)
                .as_str()
                .ends_with("-chart")
        );
        assert!(
            NodeId::for_type(NodeType::QueryError)
                .as_str()
                .ends_with("-error")
        );
    }

    #[test]
    fn fresh_ids_have_prefix_and_eight_chars() {
        let id = NodeId::query();
        let (prefix, rest) = id.as_str().split_once('_').unwrap();
        assert_eq!(prefix, "query");
        assert_eq!(rest.len(), 8);
    }
}
