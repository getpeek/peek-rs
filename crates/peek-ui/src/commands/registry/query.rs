use super::super::{
    CANVAS_NOT_TYPING, Command, Group, QUERY_EDITOR, QUERY_NODE, actions, can_rerun_page,
    can_run_queries, has_selected_queries, one_selected_query,
};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Query::Run",
        title: "Run query",
        label: None,
        group: Group::Query,
        keywords: "execute sql send",
        // `cmd-enter`, the binding `QueryNode.tsx` registers inside its editor. Bound on the
        // editor so it fires while that holds focus, which is where it is pressed; the canvas
        // carries a fallback handler so the palette reaches it too.
        default_keys: &["meta-enter"],
        context: QUERY_EDITOR,
        build: || Box::new(actions::query::Run),
        available: can_run_queries,
    },
    Command {
        id: "Query::Format",
        title: "Format query",
        label: None,
        group: Group::Query,
        keywords: "pretty print indent sql",
        default_keys: &["meta-s"],
        context: QUERY_NODE,
        build: || Box::new(actions::query::Format),
        available: has_selected_queries,
    },
    Command {
        id: "Query::Focus",
        title: "Edit the selected query",
        label: None,
        group: Group::Query,
        keywords: "enter edit focus query editor sql",
        default_keys: &["enter"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::query::Focus),
        available: one_selected_query,
    },
    Command {
        id: "Query::RerunAll",
        title: "Rerun all queries on page",
        label: None,
        group: Group::Query,
        keywords: "refresh execute every all sql",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::query::RerunAll),
        available: can_rerun_page,
    },
    Command {
        id: "Query::RerunSelected",
        title: "Rerun selected queries",
        label: None,
        group: Group::Query,
        keywords: "refresh execute selection sql",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::query::RerunSelected),
        available: can_run_queries,
    },
];
