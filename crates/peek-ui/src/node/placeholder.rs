//! What a shell shows until the real node body for its kind lands. Two kinds still come
//! through here: `ResultInsertForm` and Activity.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, div, rems};
use peek_document::{Node, NodeKind, VariableValue};
use peek_theme::ActivePeekTheme;

pub(crate) fn title(node: &Node) -> String {
    match &node.kind {
        NodeKind::Query(data) => data
            .description
            .clone()
            .filter(|description| !description.trim().is_empty())
            .unwrap_or_else(|| first_line(&data.query)),
        NodeKind::Result(data) => first_line(&data.query),
        NodeKind::Text(data) => first_line(&data.text),
        NodeKind::Variable(data) => format!("{} variables", data.rows.len()),
        NodeKind::Agent(data) => first_line(&data.query),
        NodeKind::Barchart(data) => format!("{} rows", data.data.len()),
        NodeKind::QueryError(data) => first_line(&data.message),
        NodeKind::TableDefinition(data) => data.table.clone(),
        NodeKind::ResultInsertForm(data) => format!("insert into {}", data.result_node_id),
        NodeKind::Draw(data) => format!("{} points", data.points.len()),
        NodeKind::Activity(_) => "activity".to_string(),
        NodeKind::Unknown => "unknown".to_string(),
    }
}

fn body_text(node: &Node) -> String {
    match &node.kind {
        NodeKind::Query(data) => preview(&data.query),
        NodeKind::Result(data) => preview(&data.query),
        NodeKind::Text(data) => preview(&data.text),
        NodeKind::Variable(data) => data
            .rows
            .iter()
            .take(4)
            .map(|row| match &row.value {
                VariableValue::One(value) => format!("{} = {value}", row.name),
                VariableValue::Many(values) => format!("{} = [{}]", row.name, values.join(", ")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        NodeKind::Agent(data) => format!("{} messages", data.messages.len()),
        NodeKind::Barchart(data) => format!("{:?}", data.chart_type),
        NodeKind::QueryError(data) => preview(&data.query),
        NodeKind::TableDefinition(data) => format!("{} columns", data.columns.len()),
        NodeKind::ResultInsertForm(_)
        | NodeKind::Draw(_)
        | NodeKind::Activity(_)
        | NodeKind::Unknown => String::new(),
    }
}

pub(crate) fn body(node: &Node, cx: &mut App) -> AnyElement {
    let theme = cx.peek_theme();
    div()
        .size_full()
        .p(rems(0.75))
        .text_size(rems(0.75))
        .text_color(theme.fg_muted)
        .child(body_text(node))
        .into_any_element()
}

pub(crate) fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("untitled")
        .to_string()
}

fn preview(text: &str) -> String {
    text.lines().take(6).collect::<Vec<_>>().join("\n")
}
