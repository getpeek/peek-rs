use super::super::{CANVAS_NOT_TYPING, Command, Group, actions, has_selected_results};

pub(super) static ENTRIES: &[Command] = &[Command {
    id: "Result::Pivot",
    title: "Pivot result",
    label: None,
    group: Group::Result,
    keywords: "transpose record view unpivot",
    default_keys: &["shift-p"],
    context: CANVAS_NOT_TYPING,
    build: || Box::new(actions::result::Pivot),
    available: has_selected_results,
}];
