//! The panel's list: the search row, the workspace groups, and the footer.
//!
//! Sizes are the reference's own, from `~/labs/peek/src/Connection/workspacePicker.css`, and
//! they are pixels rather than rems for the reason the rest of the chrome is: the reference's
//! panel does not scale with a root font size, and at Peek's 13 px rem the nearest component
//! step lands a few pixels out on every row.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, FontWeight, Hsla, Pixels, SharedString, Window, div, px, rems, transparent_black,
};
use peek_theme::ActivePeekTheme;

use super::entry::{self, Entry};
use super::{PickerView, View};

/// `.picker-popover` is 460 px wide; the gap under the pill is the reference's `my={8}`.
const WIDTH: Pixels = px(460.0);
const GAP_UNDER_PILL: Pixels = px(8.0);
/// `max-height: 70vh`. The window minimum is 760 × 480, so this never collapses the list away.
const MAX_HEIGHT_RATIO: f32 = 0.7;
/// `.picker-conn-dot` and `.picker-ws-mascot`.
const DOT: Pixels = px(7.0);
const MASCOT: Pixels = px(26.0);

/// The panel, anchored under the pill. Right-aligned on the title bar's own right padding, so
/// it tracks the pill without measuring it.
pub(super) fn panel(
    view: &PickerView,
    window: &mut Window,
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let theme = cx.peek_theme();
    let height = window.viewport_size().height * MAX_HEIGHT_RATIO;
    let rows = view.rows(cx);
    let searching = view.is_searching(cx);

    div()
        .id("connection-picker-panel")
        .test_support()
        .absolute()
        .top(super::super::HEIGHT + GAP_UNDER_PILL)
        .right(rems(0.75))
        .w(WIDTH)
        .max_h(height)
        .occlude()
        .v_flex()
        .overflow_hidden()
        .bg(theme.node_bg)
        .border_1()
        .border_color(theme.node_border)
        .rounded(theme.radius_card)
        .when_some(theme.node_shadow, |this, (blur, spread, color)| {
            this.shadow(vec![gpui_kit::BoxShadow {
                color,
                offset: gpui_kit::point(px(0.0), px(4.0)),
                blur_radius: blur,
                spread_radius: spread,
                inset: false,
            }])
        })
        .map(|panel| match view.view() {
            Some(View::Connection(form)) => {
                panel.child(super::connection_form::body(form, cx).into_any_element())
            }
            Some(View::Workspace(form)) => {
                panel.child(super::workspace_form::body(form, cx).into_any_element())
            }
            _ => panel
                .child(search_row(view, cx).into_any_element())
                .child(list(view, rows, searching, cx).into_any_element())
                .child(footer(cx).into_any_element()),
        })
}

/// `.picker-search`: a magnifier and a borderless field.
fn search_row(view: &PickerView, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .gap(px(10.0))
        .pt(px(14.0))
        .px(px(18.0))
        .pb(px(10.0))
        .child(
            Icon::new(IconName::Search)
                .size(px(14.0))
                .text_color(theme.fg_subtle),
        )
        .children(view.query().map(|query| {
            div()
                .id("connection-picker-search")
                .test_support()
                .flex_1()
                .min_w_0()
                .child(Input::new(query).appearance(false))
        }))
}

/// `.picker-list`, and the empty states. There are two, and they say different things: nothing
/// configured at all, against nothing matching what was typed.
fn list(
    view: &PickerView,
    rows: Vec<Entry>,
    searching: bool,
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let theme = cx.peek_theme();
    let cursor = view.cursor_entry(cx);
    let open = view.open_connection().cloned();

    let body = div()
        .id("connection-picker-list")
        .v_flex()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .pt(px(2.0))
        .px(px(10.0))
        .pb(px(8.0));

    if rows.is_empty() {
        let message = if searching {
            "No workspaces or connections match."
        } else {
            "No workspaces yet. Create your first one below."
        };
        return body.child(
            div()
                .py(px(18.0))
                .w_full()
                .text_center()
                .text_xs()
                .text_color(theme.fg_subtle)
                .child(message),
        );
    }

    let mut groups = Vec::new();
    for rows in entry::group(rows) {
        groups.push(group(view, rows, (&cursor, &open), cx).into_any_element());
    }
    body.children(groups)
}

/// One workspace: its header, and its connections when it is showing them.
fn group(
    view: &PickerView,
    group: (SharedString, Vec<Entry>),
    marks: (&Option<Entry>, &Option<(SharedString, SharedString)>),
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let (workspace, rows) = group;
    let (cursor, open) = marks;
    // The cursor's own workspace is always showing: the reference expands whatever the arrow
    // keys walk into, so a collapsed group can never hide the row Enter would pick.
    let holds_cursor = cursor
        .as_ref()
        .is_some_and(|entry| entry.workspace == workspace);
    let expanded = view.is_expanded(&workspace, view.is_searching(cx)) || holds_cursor;

    let count = rows.len();
    let mut children = Vec::new();
    if expanded {
        for entry in rows {
            children.push(row(&entry, (cursor.as_ref(), open.as_ref()), cx).into_any_element());
        }
    }

    let searching = view.is_searching(cx);
    div()
        .v_flex()
        .child(header(&workspace, (count, expanded), cx))
        .when(expanded, |this| {
            this.child(
                div()
                    .v_flex()
                    .gap(px(2.0))
                    .pt(px(4.0))
                    .pb(px(6.0))
                    .pl(px(14.0))
                    .children(children)
                    // Hidden while searching, as the reference has it: a row that is not a
                    // result has no business in a list of results.
                    .children((!searching).then(|| add_row(&workspace, cx))),
            )
        })
}

/// `.picker-add-row`: the plus tile and its label, per workspace.
fn add_row(workspace: &SharedString, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    let target = workspace.clone();
    div()
        .id(SharedString::from(format!(
            "connection-picker-add-{workspace}"
        )))
        .test_support()
        .h_flex()
        .gap(px(8.0))
        .items_center()
        .py(px(8.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .text_size(px(12.5))
        .text_color(theme.fg_subtle)
        .hover(move |style| style.bg(theme.fg.opacity(0.04)))
        .child(plus_tile(theme.fg.opacity(0.05), cx))
        .child("Add connection")
        .on_click(cx.listener(move |this, _, window, cx| {
            this.edit_connection((&target, None), window, cx);
        }))
}

/// The 18 px rounded plus the reference puts in front of both add affordances.
fn plus_tile(background: Hsla, cx: &mut Context<PickerView>) -> impl IntoElement {
    let color = cx.peek_theme().fg_subtle;
    div()
        .size(px(18.0))
        .flex_shrink_0()
        .rounded(px(5.0))
        .bg(background)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Plus).size(px(12.0)).text_color(color))
}

/// `.picker-ws-head`: the letter tile, the name, the count, the chevron.
fn header(
    workspace: &SharedString,
    state: (usize, bool),
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let (count, expanded) = state;
    let (fg, fg_subtle, tile) = {
        let theme = cx.peek_theme();
        (theme.fg, theme.fg_subtle, theme.node_bg_2)
    };
    let hover = fg.opacity(0.04);
    let mascot = workspace
        .chars()
        .next()
        .map_or_else(|| "·".to_string(), |first| first.to_uppercase().to_string());
    let name = workspace.clone();

    div()
        .id(SharedString::from(format!(
            "connection-picker-workspace-{workspace}"
        )))
        .test_support()
        .aria_expanded(expanded)
        .h_flex()
        .gap(px(12.0))
        .py(px(9.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .size(MASCOT)
                .flex_shrink_0()
                .rounded(px(7.0))
                .bg(tile)
                .flex()
                .items_center()
                .justify_center()
                .font_family("Monaspace Krypton")
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(mascot),
        )
        .child(
            div()
                .h_flex()
                .gap(px(8.0))
                .items_baseline()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg)
                        .truncate()
                        .child(name.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(fg_subtle)
                        .flex_shrink_0()
                        .child(connection_count(count)),
                ),
        )
        .child(workspace_actions(workspace, cx))
        .child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(px(12.0))
            .text_color(fg_subtle),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_workspace(&name, cx)))
}

/// Renaming and removing a workspace, both through its form — same reasoning as the row's.
fn workspace_actions(workspace: &SharedString, cx: &mut Context<PickerView>) -> impl IntoElement {
    let target = workspace.clone();
    Button::new(SharedString::from(format!(
        "connection-picker-edit-workspace-{workspace}"
    )))
    .ghost()
    .xsmall()
    .icon(Icon::new(IconName::Pencil))
    .tooltip("Edit workspace")
    .on_click(cx.listener(move |this, _, window, cx| {
        // The header underneath collapses the group.
        cx.stop_propagation();
        this.edit_workspace(Some(&target), window, cx);
    }))
}

/// "1 connection" against "3 connections" — the reference singularises, and a count that reads
/// wrong is the kind of thing people notice every single time.
fn connection_count(count: usize) -> String {
    if count == 1 {
        "1 connection".to_string()
    } else {
        format!("{count} connections")
    }
}

/// `.picker-conn`: the tint dot, the name with its SSH badge, and the `user@host` line.
fn row(
    entry: &Entry,
    marks: (Option<&Entry>, Option<&(SharedString, SharedString)>),
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let (cursor, open) = marks;
    // Copied out rather than held: `cx.peek_theme()` borrows the context the listeners below
    // need mutably, and every role here is an `Hsla`, which is `Copy`.
    let (fg, fg_muted, fg_subtle) = {
        let theme = cx.peek_theme();
        (theme.fg, theme.fg_muted, theme.fg_subtle)
    };
    let tint = super::super::connection::resolve(entry.tint).unwrap_or(fg_subtle);
    let under_cursor = cursor.is_some_and(|at| at == entry);
    let is_open = open.is_some_and(|(workspace, connection)| {
        &entry.workspace == workspace && &entry.name == connection
    });

    let (workspace, name) = (entry.workspace.clone(), entry.name.clone());
    let origin = entry.origin();

    div()
        .id(SharedString::from(format!(
            "connection-picker-row-{}/{}",
            entry.workspace, entry.name
        )))
        .test_support()
        .aria_selected(under_cursor)
        .h_flex()
        .gap(px(10.0))
        .items_start()
        .py(px(9.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .text_color(if is_open { fg } else { fg_muted })
        .when(is_open, |this| this.bg(tint.opacity(0.10)))
        .when(under_cursor, |this| {
            this.bg(tint.opacity(if is_open { 0.14 } else { 0.03 }))
                .border_1()
                .border_color(tint.opacity(0.30))
        })
        // The border only exists under the cursor, so every other row needs the same inset or
        // the text jogs sideways as the cursor passes.
        .when(!under_cursor, |this| {
            this.border_1().border_color(transparent_black())
        })
        .child(
            div()
                .mt(px(5.0))
                .size(DOT)
                .flex_shrink_0()
                .rounded_full()
                .bg(tint.opacity(if is_open || under_cursor { 1.0 } else { 0.7 })),
        )
        .child(
            div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .h_flex()
                        .gap(px(6.0))
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(if is_open {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .truncate()
                                .child(highlighted(&entry.name, &entry.name_match, (fg, tint))),
                        )
                        .when(entry.tunnelled, |this| this.child(ssh_badge(fg_subtle))),
                )
                .children(origin.map(|origin| {
                    div()
                        .font_family("Monaspace Krypton")
                        .text_size(px(11.5))
                        .text_color(fg_subtle)
                        .truncate()
                        .child(highlighted(&origin, &entry.origin_match, (fg, tint)))
                })),
        )
        .child(row_actions((&entry.workspace, &entry.name), cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.switch_to(&workspace, &name, window, cx);
        }))
}

/// `.picker-conn-actions`: Edit and Duplicate for one connection.
///
/// Inline buttons rather than the reference's "…" popup menu. A menu here would be an overlay
/// opened from inside an overlay, which is the one thing the design guide says not to stack,
/// and two affordances do not need a menu to hold them. Remove lives in the form, where the
/// confirmation can sit next to the thing it is about to remove — which is where the reference
/// puts it too, its menu item only opening the same form.
fn row_actions(
    at: (&SharedString, &SharedString),
    cx: &mut Context<PickerView>,
) -> impl IntoElement {
    let edit = (at.0.clone(), at.1.clone());
    let duplicate = (at.0.clone(), at.1.clone());
    // Duplicate writes immediately, so it is gated like every other write. Edit only opens a
    // form, whose own Save carries the gate.
    let writable = crate::settings::Settings::can_write(cx);

    div()
        .h_flex()
        .gap(px(2.0))
        .flex_shrink_0()
        .child(
            Button::new(SharedString::from(format!(
                "connection-picker-edit-{}/{}",
                at.0, at.1
            )))
            .ghost()
            .xsmall()
            .icon(Icon::new(IconName::Pencil))
            .tooltip("Edit connection")
            .on_click(cx.listener(move |this, _, window, cx| {
                // The row underneath would otherwise switch to the connection being edited.
                cx.stop_propagation();
                this.edit_connection((&edit.0, Some(&edit.1)), window, cx);
            })),
        )
        .child(
            Button::new(SharedString::from(format!(
                "connection-picker-duplicate-{}/{}",
                at.0, at.1
            )))
            .ghost()
            .xsmall()
            .icon(Icon::new(IconName::Copy))
            .disabled(!writable)
            .tooltip(if writable {
                "Duplicate connection"
            } else {
                "Read-only: relaunch with --write to change settings.json"
            })
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.stop_propagation();
                PickerView::duplicate_connection((&duplicate.0, &duplicate.1), cx);
            })),
        )
}

/// `.picker-conn-env`: a terminal glyph and the word SSH, only when a tunnel is configured.
fn ssh_badge(color: Hsla) -> impl IntoElement {
    div()
        .h_flex()
        .gap(px(3.0))
        .flex_shrink_0()
        .text_color(color)
        .child(Icon::new(IconName::Terminal).size(px(10.0)))
        .child(
            div()
                .font_family("Monaspace Krypton")
                .text_size(px(9.5))
                .font_weight(FontWeight::MEDIUM)
                .child("SSH"),
        )
}

/// The matched characters, bolder and underlined in the connection's own tint — the reference's
/// `mark.match`, whose underline colour is `var(--cdot)`.
///
/// Runs are coalesced so a contiguous match is one element rather than one per character.
fn highlighted(text: &SharedString, matched: &[usize], colors: (Hsla, Hsla)) -> impl IntoElement {
    let (foreground, tint) = colors;
    if matched.is_empty() {
        return div().child(text.clone());
    }

    let mut spans = Vec::new();
    for (index, character) in text.chars().enumerate() {
        let hit = matched.contains(&index);
        match spans.last_mut() {
            Some((was_hit, run)) if *was_hit == hit => *run = format!("{run}{character}"),
            _ => spans.push((hit, character.to_string())),
        }
    }

    div()
        .h_flex()
        .children(spans.into_iter().map(move |(hit, run)| {
            div()
                .when(hit, |this| {
                    this.font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .underline()
                        .text_decoration_color(tint)
                })
                .child(run)
        }))
}

/// `.picker-foot`. Both halves earn their place: the left one acts on the list above it, and
/// the right one names the key that acts on the cursor.
fn footer(cx: &mut Context<PickerView>) -> impl IntoElement {
    let (fg, fg_subtle, border) = {
        let theme = cx.peek_theme();
        (theme.fg, theme.fg_subtle, theme.node_border)
    };
    div()
        .h_flex()
        .justify_between()
        .items_center()
        .pt(px(8.0))
        .px(px(12.0))
        .pb(px(10.0))
        .border_t_1()
        .border_color(border)
        .bg(fg.opacity(0.02))
        .child(
            div()
                .id("connection-picker-new-workspace")
                .test_support()
                .h_flex()
                .gap(px(8.0))
                .items_center()
                .py(px(4.0))
                .px(px(6.0))
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(fg_subtle)
                .hover(move |style| style.bg(fg.opacity(0.04)))
                .child(plus_tile(fg.opacity(0.08), cx))
                .child("New workspace")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.edit_workspace(None, window, cx);
                })),
        )
        .child(
            div()
                .h_flex()
                .gap(px(6.0))
                .text_size(px(10.5))
                .font_family("Monaspace Krypton")
                .text_color(fg_subtle)
                .child("\u{21b5}")
                .child("switch"),
        )
}

/// A form's header: back to the list, what is being edited, and the way out.
///
/// The back control is a `Button`, not a link: it is an in-app command, and the design guide
/// reserves link styling for a resource that opens in a browser.
pub(super) fn form_header(title: SharedString, cx: &mut Context<PickerView>) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .items_center()
        .gap(px(8.0))
        .pt(px(12.0))
        .px(px(12.0))
        .pb(px(10.0))
        .border_b_1()
        .border_color(theme.node_border)
        .child(
            Button::new("connection-form-back")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::ChevronLeft))
                .label("Workspaces")
                .on_click(cx.listener(|this, _, window, cx| this.back_to_list(window, cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .text_color(theme.fg_subtle)
                .child(title),
        )
        .child(
            Button::new("connection-form-close")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::X))
                .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
        )
}
