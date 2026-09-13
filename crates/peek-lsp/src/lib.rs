#[cfg(test)]
mod _dump_tree;
mod backend;
mod completion;
mod context;
mod diagnostics;
mod documents;
mod highlights;
mod parser;
mod position;
mod query_info;
mod schema;
mod scope;
mod sql_text;

/// The LSP vocabulary this crate speaks, re-exported so callers share one `lsp-types`
/// without depending on it directly. gpui-base pins the same version, which is what lets
/// a `CompletionItem` from here go straight into its completion menu.
pub use lsp_types;

pub use backend::Backend;
pub use completion::anchor_to_typed_prefix;
pub use highlights::sql_highlights;
pub use parser::sql_language;
pub use query_info::{QueryInfo, StatementType, TableRef, analyze as analyze_query};
pub use schema::{SchemaIndex, SharedSchema, set_schema, shared_schema};
pub use sql_text::{
    Substitution, VariableSite, format, is_unbounded_write, substitute, variable_prefix_at,
    variable_sites,
};
