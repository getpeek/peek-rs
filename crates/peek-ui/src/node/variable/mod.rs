//! The Variable node: `~/labs/peek/src/canvas/nodes/Variable/VariableNode.tsx`.
//!
//! A three-column table over `VariableData.rows` — `@name`, value, row actions — above a
//! footer that adds rows and marks the node global. The document is the only source of truth:
//! every field pushes its change through [`Document::update_data`] as it is typed, and
//! [`VariableEditor::reconcile`] pulls the rows back out on the way to the screen, so undo,
//! the palette and (later) multiplayer all reach the same rows the user is editing.
//!
//! Three deliberate departures from the reference:
//!
//! - the list editor expands inline instead of in a popover, because gpui lays overlays out
//!   outside the canvas' rem scope and a popover would ignore the camera's zoom;
//! - that editor has no line-number gutter (the reference mirrors the textarea's `scrollTop`
//!   onto a parallel column, which needs a scroll position gpui's input does not publish);
//! - the four source handles are not here. Edges are drawn and selectable, but dragging one
//!   into being is not implemented, and the handles are that gesture's only affordance.

mod name;
mod quoting;

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{
    Escape, Input, InputEvent, InputState, Paste, Textarea, TextareaState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Selectable, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, SharedString, Subscription, Window, div,
    relative, rems,
};
use peek_canvas::Document;
use peek_document::{NodeData, NodeId, NodeType, VariableData, VariableRow, VariableValue};
use peek_theme::ActivePeekTheme;

use super::kind::NodeContext;
use super::state::NodeState;
use name::NameProblem;

/// `variable-col-actions`, the width of the two row buttons.
const ACTIONS_WIDTH: f32 = 3.5;
/// `variable-col-name`.
const NAME_FRACTION: f32 = 0.4;

/// Retained editing state: the node's whole body, kept alive across frames so the inputs keep
/// their text, focus and carets while the camera moves.
#[derive(Debug)]
pub(crate) struct VariableState {
    editor: Entity<VariableEditor>,
}

impl VariableState {
    pub(crate) fn new(
        id: &NodeId,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let node = id.clone();
        let document = document.clone();
        Self {
            editor: cx.new(|cx| VariableEditor::new(node, document, window, cx)),
        }
    }
}

/// `headerName`: only named rows count, so a node of blank rows still reads "no variables".
pub(crate) fn title(data: &VariableData) -> String {
    match data.rows.iter().filter(|row| !row.name.is_empty()).count() {
        0 => "no variables".to_string(),
        1 => "1 variable".to_string(),
        count => format!("{count} variables"),
    }
}

pub(crate) fn body(
    _id: &NodeId,
    data: &VariableData,
    context: NodeContext<'_>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(NodeState::Variable(state)) = context.state else {
        return div().into_any_element();
    };
    let editor = state.editor.clone();
    editor.update(cx, |editor, cx| editor.reconcile(data, window, cx));
    editor.into_any_element()
}

/// The reference keeps every control in the body and the footer; the header carries only the
/// row count the shell already draws.
pub(crate) fn header_extras(
    _id: &NodeId,
    _data: &VariableData,
    _context: NodeContext<'_>,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyElement> {
    None
}

#[derive(Debug)]
struct VariableEditor {
    node: NodeId,
    document: Entity<Document>,
    focus_handle: FocusHandle,
    rows: Vec<RowEditor>,
    /// Rows have no identity on disk, so element ids and pending edits are keyed by a counter
    /// that survives the removal of the rows above them.
    next_key: usize,
    /// The row whose list editor is open, if any.
    expanded: Option<usize>,
    /// The document revision the fields were last brought in line with. A keystroke reaches
    /// the input before the `Change` event that carries it to the document, and rendering
    /// happens in between: without this, that frame would find the two out of step and push
    /// the older document text back over what was just typed.
    revision: u64,
}

/// One row's three fields. `value` and `lines` both exist for the life of the row: which one
/// is shown follows the `VariableValue` in the document, and a row flips between them often
/// enough that keeping the other's text is worth an idle entity.
#[derive(Debug)]
struct RowEditor {
    key: usize,
    name: Entity<InputState>,
    value: Entity<InputState>,
    lines: Entity<TextareaState>,
    /// Held only for their `Drop`: these are what push typing into the document, and they
    /// must die with the row they belong to.
    _subscriptions: Vec<Subscription>,
}

/// Which field of which row an [`InputEvent`] came from.
#[derive(Debug, Clone, Copy)]
struct Field {
    key: usize,
    kind: FieldKind,
}

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    Name,
    Value,
    Lines,
}

/// Everything one row needs to draw itself, so [`VariableEditor::row`] does not take the
/// document, the theme and the sibling names as five separate parameters.
#[derive(Debug, Clone, Copy)]
struct RowView<'a> {
    data: &'a VariableRow,
    editor: &'a RowEditor,
    problem: Option<NameProblem>,
}

fn blank_row() -> VariableRow {
    VariableRow {
        name: String::new(),
        value: VariableValue::One(String::new()),
    }
}

fn as_lines(value: &VariableValue) -> String {
    match value {
        VariableValue::One(value) => value.clone(),
        VariableValue::Many(values) => values.join("\n"),
    }
}

impl RowEditor {
    fn new(
        key: usize,
        row: &VariableRow,
        window: &mut Window,
        cx: &mut Context<VariableEditor>,
    ) -> Self {
        let single = match &row.value {
            VariableValue::One(value) => value.clone(),
            VariableValue::Many(_) => String::new(),
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("name")
                .default_value(row.name.clone())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("value")
                .default_value(single)
        });
        let lines = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("one value per line")
                .auto_grow(3, 8)
                .default_value(as_lines(&row.value))
        });
        let subscriptions = vec![
            subscribe(&name, FieldKind::Name, key, cx),
            subscribe(&value, FieldKind::Value, key, cx),
            subscribe_lines(&lines, key, cx),
        ];
        Self {
            key,
            name,
            value,
            lines,
            _subscriptions: subscriptions,
        }
    }
}

fn subscribe(
    state: &Entity<InputState>,
    kind: FieldKind,
    key: usize,
    cx: &mut Context<VariableEditor>,
) -> Subscription {
    cx.subscribe(state, move |editor, _, event, cx| {
        editor.on_field_event(Field { key, kind }, event, cx);
    })
}

fn subscribe_lines(
    state: &Entity<TextareaState>,
    key: usize,
    cx: &mut Context<VariableEditor>,
) -> Subscription {
    cx.subscribe(state, move |editor, _, event, cx| {
        editor.on_field_event(
            Field {
                key,
                kind: FieldKind::Lines,
            },
            event,
            cx,
        );
    })
}

impl VariableEditor {
    fn new(
        node: NodeId,
        document: Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let document_revision = document.read(cx).revision();
        let mut editor = Self {
            node,
            document,
            focus_handle: cx.focus_handle(),
            rows: Vec::new(),
            next_key: 0,
            expanded: None,
            revision: document_revision,
        };
        for row in editor.document_rows(cx) {
            editor.push_row(&row, window, cx);
        }
        editor
    }

    fn document_rows(&self, cx: &App) -> Vec<VariableRow> {
        self.data(cx)
            .map(|data| data.rows.clone())
            .unwrap_or_default()
    }

    fn data<'a>(&self, cx: &'a App) -> Option<&'a VariableData> {
        self.document
            .read(cx)
            .node(&self.node)
            .and_then(|node| VariableData::get(&node.kind))
    }

    fn is_global(&self, cx: &App) -> bool {
        self.data(cx).and_then(|data| data.is_global) == Some(true)
    }

    fn index_of(&self, key: usize) -> Option<usize> {
        self.rows.iter().position(|row| row.key == key)
    }

    fn push_row(&mut self, row: &VariableRow, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.next_key;
        self.next_key += 1;
        self.rows.push(RowEditor::new(key, row, window, cx));
    }

    fn update_rows(&self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Vec<VariableRow>)) {
        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            if document.update_data::<VariableData>(&node, |data| edit(&mut data.rows)) {
                cx.notify();
            }
        });
    }

    /// Brings the fields back in line with the document, `useSyncedFieldValue`'s job. The
    /// handlers below keep [`Self::rows`] in step with their own edits, so the only changes
    /// that reach here are the ones from elsewhere: an undo, later a remote edit. Those are
    /// matched by position — rows carry no identity on disk — so an external removal moves
    /// the text up a row rather than moving the row.
    fn reconcile(&mut self, data: &VariableData, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.document.read(cx).revision();
        if revision == self.revision {
            return;
        }
        self.revision = revision;
        self.rows.truncate(data.rows.len());
        while self.rows.len() < data.rows.len() {
            let row = data.rows[self.rows.len()].clone();
            self.push_row(&row, window, cx);
        }
        for (editor, row) in self.rows.iter().zip(&data.rows) {
            if editor.name.read(cx).value().as_ref() != row.name {
                editor.name.update(cx, |state, cx| {
                    state.set_value(row.name.clone(), window, cx);
                });
            }
            let text = as_lines(&row.value);
            match &row.value {
                VariableValue::One(_) if editor.value.read(cx).value().as_ref() != text => editor
                    .value
                    .update(cx, |state, cx| state.set_value(text, window, cx)),
                VariableValue::Many(_) if editor.lines.read(cx).value().as_ref() != text => editor
                    .lines
                    .update(cx, |state, cx| state.set_value(text, window, cx)),
                _ => {}
            }
        }
    }

    // ---- edits -------------------------------------------------------------------------

    fn on_field_event(&mut self, field: Field, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => self.commit(field, cx),
            // `checkpoint` seals the undo transaction a run of keystrokes opened, which is
            // what makes one visit to a field one undo step.
            InputEvent::Blur => self
                .document
                .update(cx, |document, _| document.checkpoint()),
            InputEvent::Focus | InputEvent::PressEnter { .. } => {}
        }
    }

    fn commit(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(field.key) else {
            return;
        };
        let editor = &self.rows[index];
        let text = match field.kind {
            FieldKind::Name => editor.name.read(cx).value(),
            FieldKind::Value => editor.value.read(cx).value(),
            FieldKind::Lines => editor.lines.read(cx).value(),
        }
        .to_string();
        self.update_rows(cx, |rows| {
            let Some(row) = rows.get_mut(index) else {
                return;
            };
            match field.kind {
                FieldKind::Name => row.name = text,
                FieldKind::Value => row.value = VariableValue::One(text),
                // The list editor stores a line per row, trailing blank included:
                // `substituteVariables` drops those, so typing a newline does not erase itself.
                FieldKind::Lines => {
                    row.value = VariableValue::Many(text.split('\n').map(str::to_string).collect());
                }
            }
        });
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_rows(cx, |rows| rows.push(blank_row()));
        self.push_row(&blank_row(), window, cx);
        cx.notify();
    }

    fn remove_row(&mut self, key: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(key) else {
            return;
        };
        self.update_rows(cx, |rows| {
            rows.remove(index);
            // `removeRow`: the node always keeps a row to type into.
            if rows.is_empty() {
                rows.push(blank_row());
            }
        });
        self.rows.remove(index);
        if self.rows.is_empty() {
            self.push_row(&blank_row(), window, cx);
        }
        if self.expanded == Some(key) {
            self.expanded = None;
        }
        self.document
            .update(cx, |document, _| document.checkpoint());
        cx.notify();
    }

    /// `toggleArrayMode`: a list joins back into one newline-separated value, and a value
    /// splits into lines — except an empty one, which becomes an empty list rather than a
    /// list holding one empty string.
    fn toggle_array(&mut self, key: usize, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(key) else {
            return;
        };
        self.update_rows(cx, |rows| {
            let Some(row) = rows.get_mut(index) else {
                return;
            };
            row.value = match &row.value {
                VariableValue::Many(values) => VariableValue::One(values.join("\n")),
                VariableValue::One(value) if value.is_empty() => VariableValue::Many(Vec::new()),
                VariableValue::One(value) => {
                    VariableValue::Many(value.split('\n').map(str::to_string).collect())
                }
            };
        });
        self.expanded = None;
        self.document
            .update(cx, |document, _| document.checkpoint());
        cx.notify();
    }

    fn toggle_expanded(&mut self, key: usize, cx: &mut Context<Self>) {
        self.expanded = if self.expanded == Some(key) {
            None
        } else {
            Some(key)
        };
        cx.notify();
    }

    fn quote_all(&mut self, key: usize, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(key) else {
            return;
        };
        self.update_rows(cx, |rows| {
            let Some(row) = rows.get_mut(index) else {
                return;
            };
            if let VariableValue::Many(values) = &row.value {
                row.value = VariableValue::Many(quoting::quote_all(values));
            }
        });
        self.document
            .update(cx, |document, _| document.checkpoint());
        cx.notify();
    }

    /// Turning `isGlobal` on connects this node to every query node on the page, exactly as
    /// the reference's footer does; `Document::connect` is idempotent, and `create_node`
    /// makes the same edge for query nodes added later.
    fn toggle_global(&mut self, cx: &mut Context<Self>) {
        let next = !self.is_global(cx);
        let node = self.node.clone();
        self.document.update(cx, |document, cx| {
            document.update_data::<VariableData>(&node, |data| data.is_global = Some(next));
            if next {
                let queries: Vec<NodeId> = document
                    .nodes()
                    .iter()
                    .filter(|node| node.node_type() == Some(NodeType::Query))
                    .map(|node| node.id.clone())
                    .collect();
                for query in &queries {
                    document.connect(&node, query);
                }
            }
            document.checkpoint();
            cx.notify();
        });
        cx.notify();
    }

    /// `VariableTextInput.onPasteLines`: a column copied out of a spreadsheet or a result grid
    /// is a list, and a single-line input would flatten it into one unusable line. This runs
    /// in the capture phase because the input strips the newlines before anyone else sees the
    /// clipboard; anything that is not a list is left to paste normally.
    fn paste_lines(&mut self, key: usize, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(key) else {
            return;
        };
        let Some(pasted) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let state = self.rows[index].value.read(cx);
        let range = state.selected_range();
        let current = state.value().to_string();
        let merged = format!(
            "{}{pasted}{}",
            &current[..range.start],
            &current[range.end..]
        );
        let lines: Vec<String> = merged
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect();
        if lines.len() < 2 {
            return;
        }
        self.update_rows(cx, |rows| {
            if let Some(row) = rows.get_mut(index) {
                row.value = VariableValue::Many(lines);
            }
        });
        self.document
            .update(cx, |document, _| document.checkpoint());
        cx.stop_propagation();
        cx.notify();
    }

    /// The input lets `Escape` propagate, and the node's own root sits inside the canvas'
    /// focus scope: moving focus here drops the `Input` key context, which is what hands the
    /// canvas' shortcuts back without leaving the node.
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------------------

    fn row(&self, view: RowView<'_>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let key = view.editor.key;
        let invalid = view.problem;
        let is_list = matches!(view.data.value, VariableValue::Many(_));
        let expanded = self.expanded == Some(key);

        div()
            .v_flex()
            .border_b_1()
            .border_color(theme.node_border)
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .child(
                        div()
                            .id(("variable-name", key))
                            .test_support()
                            .aria_label(invalid.map_or("Variable name", NameProblem::message))
                            .h_flex()
                            .items_center()
                            .gap(rems(0.25))
                            .w(relative(NAME_FRACTION))
                            .flex_shrink_0()
                            .pl(rems(0.625))
                            // The reference only turns the text red, which says something is
                            // wrong without saying what; the tooltip carries the reason and
                            // the label carries it to the accessibility tree.
                            .when_some(invalid, |cell, problem| {
                                cell.bg(theme.red_soft).tooltip(move |window, cx| {
                                    Tooltip::new(problem.message()).build(window, cx)
                                })
                            })
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(if invalid.is_some() {
                                        theme.red
                                    } else {
                                        theme.accent_soft
                                    })
                                    .child("@"),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&view.editor.name)
                                        .id(("variable-name-input", key))
                                        .appearance(false)
                                        .xsmall(),
                                ),
                            ),
                    )
                    .child(Self::value_cell(view, cx))
                    .child(Self::row_actions(view, cx)),
            )
            .when(is_list && expanded, |row| {
                row.child(Self::list_editor(view, cx))
            })
    }

    fn value_cell(view: RowView<'_>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme().clone();
        let key = view.editor.key;
        let VariableValue::Many(values) = &view.data.value else {
            return div()
                .flex_1()
                .min_w_0()
                .px(rems(0.375))
                .capture_action(cx.listener(move |editor, _: &Paste, _, cx| {
                    editor.paste_lines(key, cx);
                }))
                .child(
                    Input::new(&view.editor.value)
                        .id(("variable-value-input", key))
                        .appearance(false)
                        .xsmall(),
                )
                .into_any_element();
        };

        let filled = values.iter().filter(|line| !line.trim().is_empty()).count();
        let label = match filled {
            0 => "empty".to_string(),
            1 => "1 value".to_string(),
            count => format!("{count} values"),
        };
        let unquoted = quoting::unquoted(values);
        div()
            .h_flex()
            .items_center()
            .gap(rems(0.3125))
            .flex_1()
            .min_w_0()
            .px(rems(0.375))
            .child(
                Button::new(("variable-list-chip", key))
                    .ghost()
                    .xsmall()
                    .p_0()
                    .child(
                        div()
                            .flex_none()
                            .px(rems(0.625))
                            .py(rems(0.125))
                            .rounded_full()
                            .bg(theme.accent_bg)
                            .text_size(rems(0.6875))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(if filled > 0 {
                                theme.accent
                            } else {
                                theme.fg_subtle
                            })
                            .child(label),
                    )
                    .tooltip("Edit the list")
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.toggle_expanded(key, cx);
                    })),
            )
            // `variable-array-chip-warn`: surfaces on the collapsed chip that some line will
            // not survive substitution, so the problem is visible without opening the editor.
            .when(unquoted > 0, |cell| {
                cell.child(
                    div()
                        .size(rems(0.3125))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(theme.yellow),
                )
            })
            .into_any_element()
    }

    fn row_actions(view: RowView<'_>, cx: &mut Context<Self>) -> impl IntoElement {
        let key = view.editor.key;
        let is_list = matches!(view.data.value, VariableValue::Many(_));
        div()
            .h_flex()
            .items_center()
            .justify_end()
            .gap(rems(0.125))
            .w(rems(ACTIONS_WIDTH))
            .flex_shrink_0()
            .pr(rems(0.375))
            .child(
                Button::new(("variable-list-toggle", key))
                    .ghost()
                    .xsmall()
                    .compact()
                    .selected(is_list)
                    .icon(IconName::Brackets)
                    .tooltip(if is_list {
                        "Convert to a single value"
                    } else {
                        "Convert to a list"
                    })
                    .on_click(cx.listener(move |editor, _, _, cx| editor.toggle_array(key, cx))),
            )
            .child(
                Button::new(("variable-remove-row", key))
                    .ghost()
                    .xsmall()
                    .compact()
                    .icon(IconName::Close)
                    .tooltip("Remove row")
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        editor.remove_row(key, window, cx);
                    })),
            )
    }

    fn list_editor(view: RowView<'_>, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let key = view.editor.key;
        let values = match &view.data.value {
            VariableValue::Many(values) => values.clone(),
            VariableValue::One(_) => Vec::new(),
        };
        let filled = values.iter().filter(|line| !line.trim().is_empty()).count();
        let unquoted = quoting::unquoted(&values);
        let name = if view.data.name.is_empty() {
            "@unnamed".to_string()
        } else {
            format!("@{}", view.data.name)
        };

        div()
            .id(("variable-list-editor", key))
            .test_support()
            .aria_label(name.clone())
            .v_flex()
            .border_t_1()
            .border_color(theme.node_border)
            .bg(theme.node_inset)
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .gap(rems(0.625))
                    .px(rems(0.625))
                    .py(rems(0.4375))
                    .bg(theme.node_bg_2)
                    .child(div().text_color(theme.accent_soft).child(name))
                    .child(div().text_color(theme.fg_subtle).child(match filled {
                        1 => "1 value".to_string(),
                        count => format!("{count} values"),
                    })),
            )
            .child(Textarea::new(&view.editor.lines).appearance(false))
            .when(unquoted > 0, |editor| {
                editor.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap(rems(0.375))
                        .px(rems(0.625))
                        .py(rems(0.375))
                        .border_t_1()
                        .border_color(theme.node_border)
                        .bg(theme.yellow_soft)
                        .text_color(theme.yellow)
                        .child(div().flex_1().min_w_0().child(match unquoted {
                            1 => "1 value would be inserted unquoted".to_string(),
                            count => {
                                format!("{count} values would be inserted unquoted")
                            }
                        }))
                        .child(
                            Button::new(("variable-quote-all", key))
                                .outline()
                                .xsmall()
                                .label("Quote all")
                                .on_click(
                                    cx.listener(move |editor, _, _, cx| editor.quote_all(key, cx)),
                                ),
                        ),
                )
            })
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let global = self.is_global(cx);
        div()
            .h_flex()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .px(rems(0.75))
            .py(rems(0.5))
            .border_t_1()
            .border_color(theme.node_border)
            .child(
                Button::new("variable-add-row")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .label("Add variable")
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_row(window, cx))),
            )
            .child(
                Button::new("variable-global")
                    .ghost()
                    .xsmall()
                    .compact()
                    .selected(global)
                    .icon(IconName::Globe)
                    .tooltip("Share these variables with every query on the page")
                    .on_click(cx.listener(|editor, _, _, cx| editor.toggle_global(cx))),
            )
    }
}

impl Render for VariableEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.document_rows(cx);
        div()
            .id(SharedString::from(format!("{}-body", self.node)))
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context("VariableNode")
            .on_action(cx.listener(Self::escape))
            .v_flex()
            .size_full()
            .overflow_hidden()
            .text_size(rems(0.75))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .children(self.rows.iter().zip(&rows).map(|(editor, row)| {
                        self.row(
                            RowView {
                                data: row,
                                editor,
                                problem: name::problem(&row.name, &rows),
                            },
                            cx,
                        )
                        .into_any_element()
                    })),
            )
            .child(self.footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::title;
    use peek_document::{VariableData, VariableRow, VariableValue};

    fn data(names: &[&str]) -> VariableData {
        VariableData {
            rows: names
                .iter()
                .map(|name| VariableRow {
                    name: (*name).to_string(),
                    value: VariableValue::One(String::new()),
                })
                .collect(),
            is_global: None,
        }
    }

    #[test]
    fn the_title_counts_named_rows_only() {
        assert_eq!(title(&data(&[])), "no variables");
        assert_eq!(title(&data(&["", ""])), "no variables");
        assert_eq!(title(&data(&["limit", ""])), "1 variable");
        assert_eq!(title(&data(&["limit", "offset"])), "2 variables");
    }
}
