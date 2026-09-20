//! The regions picker — the port of `RegionsMenu.tsx` and `.wf-region-list`.
//!
//! A panel above the zoom cluster listing every region on the page: click a row to fly to it,
//! rename it, or remove it. It is the only way to reach a region by name, and the only place a
//! region created by ⌘G gets one — `Region::GroupSelection` opens it with the new region in
//! rename mode rather than inventing a name and leaving it.
//!
//! Hand-owned rather than a `Popover`, for the reasons `title_bar/pages/panel.rs` records: a
//! field inside a popover never sees a space, and the press that dismisses one has to be
//! swallowed before it reaches the canvas and starts a marquee.

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, BoxShadow, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, ScrollHandle, SharedString, Subscription, WeakEntity,
    Window, div, point, px, transparent_black,
};
use peek_document::{RegionId, RegionStatus};
use peek_theme::ActivePeekTheme;

use crate::canvas::CanvasView;
use crate::canvas::dispatch::regions::Grouping;
use crate::commands::actions;

/// `.wf-region-list`'s width.
const WIDTH: Pixels = px(280.0);
const MAX_HEIGHT: Pixels = px(320.0);
/// Where the panel sits: clear of the zoom cluster it belongs to, in the same pixels the
/// cluster is pinned in — this chrome does not scale with a root font size.
const LEFT: Pixels = px(16.0);
const BOTTOM: Pixels = px(62.0);
const DOT: Pixels = px(8.0);

/// What the two sparkle buttons can do this frame. `None` when no local model is configured,
/// which is when the reference hides them rather than offering an action that cannot run.
struct Ai {
    /// The canvas's handle, because a button dispatches the same action the palette does.
    focus: FocusHandle,
    can_group_ungrouped: bool,
    can_regroup_all: bool,
    /// The grouping already in flight, if any: its button spins and the other is inert.
    running: Option<Grouping>,
}

/// One row, resolved against the live nodes so a count means what the canvas shows.
struct Row {
    id: RegionId,
    name: String,
    color_index: u8,
    suggested: bool,
    members: usize,
}

pub(crate) struct RegionsPanel {
    canvas: WeakEntity<CanvasView>,
    open: bool,
    /// The region whose name is being edited, if any. Set together with `open` by the flows
    /// that create a region and hand naming off.
    renaming: Option<RegionId>,
    rename: Entity<InputState>,
    cursor: usize,
    rows_scroll: ScrollHandle,
    /// Whatever held focus when the panel opened — the canvas, on every path that gets here.
    /// Kept rather than reaching back into the canvas entity on the way out: `close` is called
    /// from inside a `CanvasView` listener, where that entity is leased and reading it panics.
    restore_focus: Option<FocusHandle>,
    _rename: Subscription,
}

impl std::fmt::Debug for RegionsPanel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegionsPanel")
            .field("open", &self.open)
            .field("renaming", &self.renaming)
            .finish_non_exhaustive()
    }
}

impl RegionsPanel {
    pub(crate) fn new(
        canvas: WeakEntity<CanvasView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rename = cx.new(|cx| InputState::new(window, cx));
        let typing = cx.subscribe_in(&rename, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_rename(window, cx);
            }
        });
        Self {
            canvas,
            open: false,
            renaming: None,
            rename,
            cursor: 0,
            rows_scroll: ScrollHandle::default(),
            restore_focus: None,
            _rename: typing,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else {
            self.opened(0, window, cx);
        }
    }

    /// Opens with `region` in rename mode: the naming hand-off ⌘G makes, and the reason a new
    /// region is called `Region 3` for only as long as it takes to type over it.
    pub(crate) fn open_renaming(
        &mut self,
        region: RegionId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Deliberately *not* read back from the document here: this is called from inside a
        // `CanvasView` listener, so the canvas entity is leased and `rows` would panic on the
        // re-entrant borrow. `render` puts the cursor on the row being renamed instead.
        self.opened(self.cursor, window, cx);
        self.start_rename(region, name, window, cx);
    }

    fn opened(&mut self, cursor: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.cursor = cursor;
        self.restore_focus = window.focused(cx);
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.renaming = None;
        // Focus may have followed the rename field in; without handing it back the window is
        // left focused on an element that no longer renders, which silently kills every canvas
        // binding including `cmd-z`.
        self.hand_focus_back(window, cx);
        cx.notify();
    }

    fn start_rename(
        &mut self,
        region: RegionId,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.renaming = Some(region);
        let rename = self.rename.clone();
        let name = name.to_string();
        rename.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// A blank name keeps the old one: an unnamed region is unreachable in a list of names.
    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(region) = self.renaming.take() else {
            return;
        };
        let name = self.rename.read(cx).value().trim().to_string();
        if !name.is_empty() {
            self.with_document(cx, |document, cx| {
                if document.rename_region(&region, name) {
                    document.checkpoint();
                    cx.notify();
                }
            });
        }
        self.hand_focus_back(window, cx);
        cx.notify();
    }

    fn hand_focus_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.restore_focus.clone() {
            window.focus(&handle, cx);
        }
    }

    fn remove(&mut self, region: &RegionId, cx: &mut Context<Self>) {
        self.with_document(cx, |document, cx| {
            if document.remove_region(region) {
                document.checkpoint();
                cx.notify();
            }
        });
        cx.notify();
    }

    fn choose(&mut self, region: &RegionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        canvas.update(cx, |canvas, cx| canvas.fly_to_region(region, window, cx));
        self.close(window, cx);
    }

    fn with_document(
        &self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut peek_canvas::Document, &mut Context<peek_canvas::Document>),
    ) {
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        let document = canvas.read(cx).document().clone();
        document.update(cx, work);
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let Some(canvas) = self.canvas.upgrade() else {
            return Vec::new();
        };
        canvas
            .read(cx)
            .derived_regions(cx)
            .into_iter()
            .map(|region| Row {
                id: region.id,
                name: region.name,
                color_index: region.color_index,
                suggested: region.status == RegionStatus::Suggested,
                members: region.member_ids.len(),
            })
            .collect()
    }

    /// Nodes on the page that no region holds, as the reference's trailing row counts them.
    /// Drawings are annotation rather than content, so they are never "ungrouped work" —
    /// `list_regions` and the AI grouping draw the same line.
    fn ungrouped(&self, cx: &App) -> usize {
        let Some(canvas) = self.canvas.upgrade() else {
            return 0;
        };
        canvas.read(cx).document().read(cx).ungrouped_count()
    }

    /// What the AI buttons can do, or `None` without a local model to ask.
    fn ai(&self, cx: &App) -> Option<Ai> {
        let canvas = self.canvas.upgrade()?;
        let canvas = canvas.read(cx);
        let scope = canvas.scope(cx);
        if !scope.ai.local_model {
            return None;
        }
        Some(Ai {
            focus: canvas.focus_handle.clone(),
            can_group_ungrouped: peek_canvas::regions::grouping::can_extend(
                scope.regions.ungrouped,
                scope.regions.count,
            ),
            can_regroup_all: peek_canvas::regions::grouping::can_partition(scope.regions.groupable),
            running: canvas.grouping(),
        })
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 {
            return;
        }
        let moved = isize::try_from(self.cursor)
            .unwrap_or(0)
            .saturating_add(step);
        self.cursor = usize::try_from(moved).unwrap_or(0).min(count - 1);
        self.rows_scroll.scroll_to_item(self.cursor);
        cx.notify();
    }

    /// `up` / `down` / `enter` / `escape`, taken on the panel root rather than bound as actions.
    ///
    /// A single-line `InputState` registers no `MoveUp`/`MoveDown` listener, so those keystrokes
    /// find no handler and propagate up to here even while the rename field has focus. Enter is
    /// the exception — the field claims it — and arrives through `InputEvent::PressEnter`.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.step(1, cx),
            "up" => self.step(-1, cx),
            "escape" if self.renaming.is_some() => {
                self.renaming = None;
                cx.notify();
            }
            "escape" => self.close(window, cx),
            "enter" if self.renaming.is_none() => {
                if let Some(row) = self.rows(cx).get(self.cursor) {
                    let id = row.id.clone();
                    self.choose(&id, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for RegionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A closed panel still renders, as an empty layer, so it repaints on its own `notify`
        // without the canvas having to observe it too.
        let layer = div().id("regions-panel-layer").absolute().inset_0();
        if !self.open {
            return layer.invisible();
        }
        let rows = self.rows(cx);
        let ungrouped = self.ungrouped(cx);
        let ai = self.ai(cx);
        // The cursor follows the row being renamed, which is what opening straight into rename
        // mode could not set for itself.
        if let Some(renaming) = &self.renaming
            && let Some(index) = rows.iter().position(|row| &row.id == renaming)
        {
            self.cursor = index;
        }
        layer
            .child(scrim(cx))
            .child(self.panel(&rows, (ungrouped, ai.as_ref()), cx))
            .on_key_down(cx.listener(Self::on_key_down))
    }
}

impl RegionsPanel {
    /// `ungrouped` carries the count and the AI state together: both belong to the trailing
    /// row, and a fourth parameter would be one too many.
    fn panel(
        &self,
        rows: &[Row],
        ungrouped: (usize, Option<&Ai>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (count, ai) = ungrouped;
        let theme = cx.peek_theme().clone();
        div()
            .id("regions-list")
            .test_support()
            .absolute()
            .left(LEFT)
            .bottom(BOTTOM)
            .w(WIDTH)
            .max_h(MAX_HEIGHT)
            .occlude()
            .v_flex()
            .overflow_hidden()
            .bg(theme.node_bg)
            .border_1()
            .border_color(theme.node_border)
            .rounded(theme.radius_card)
            .when_some(theme.node_shadow, |this, (offset_y, blur, color)| {
                this.shadow(vec![BoxShadow {
                    color,
                    offset: point(px(0.0), offset_y),
                    blur_radius: blur,
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .child(Self::title(ai, cx))
            .child(self.list(rows, cx))
            .children((count > 0).then(|| Self::ungrouped_row((count, ai), cx)))
    }

    /// The header, with "regroup all with AI" hung off it — it reshapes the whole list below
    /// rather than any one row, which is why the reference puts it here.
    fn title(ai: Option<&Ai>, cx: &App) -> impl IntoElement {
        let theme = cx.peek_theme();
        div()
            .h_flex()
            .items_center()
            .px(px(14.0))
            .pt(px(10.0))
            .pb(px(6.0))
            .text_size(px(10.0))
            .font_family("Monaspace Krypton")
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.fg_subtle)
            .child(div().flex_1().child("REGIONS"))
            .children(ai.filter(|ai| ai.can_regroup_all).map(|ai| {
                sparkle(
                    "regions-regroup-all-ai",
                    "Regroup all with AI",
                    (Grouping::All, ai),
                    cx,
                )
            }))
    }

    fn list(&self, rows: &[Row], cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme();
        let body = div()
            .id("regions-rows")
            .track_scroll(&self.rows_scroll)
            .v_flex()
            .gap(px(1.0))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.0))
            .pb(px(4.0));

        if rows.is_empty() {
            return body.child(
                div()
                    .px(px(8.0))
                    .pb(px(8.0))
                    .text_xs()
                    .text_color(theme.fg_subtle)
                    .child("Select nodes and press \u{2318}G to group them"),
            );
        }
        body.children(
            rows.iter()
                .enumerate()
                .map(|(index, row)| self.row(row, index == self.cursor, cx)),
        )
    }

    fn row(&self, row: &Row, under_cursor: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.peek_theme().clone();
        let renaming = self.renaming.as_ref() == Some(&row.id);
        let color = theme.region(row.color_index as usize);
        let id = row.id.clone();
        let for_rename = (row.id.clone(), row.name.clone());
        let for_remove = row.id.clone();

        div()
            .id(SharedString::from(format!("region-row-{}", row.id)))
            .test_support()
            .group(SharedString::from(format!("region-row-{}", row.id)))
            .aria_selected(under_cursor)
            .h_flex()
            .gap(px(9.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(theme.radius_card)
            .when(under_cursor, |this| this.bg(theme.node_bg_2))
            .hover({
                let lit = theme.node_bg_2;
                move |style| style.bg(lit)
            })
            .text_xs()
            .text_color(theme.fg)
            .child(div().size(DOT).flex_shrink_0().rounded_full().bg(color))
            .child(if renaming {
                div()
                    .id("region-rename")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.rename).appearance(false))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(row.name.clone())
                    .into_any_element()
            })
            .children(row.suggested.then(|| suggested_badge(cx)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .font_family("Monaspace Krypton")
                    .text_color(theme.fg_subtle)
                    .child(row.members.to_string()),
            )
            .when(!renaming, |this| {
                this.child(
                    div()
                        .h_flex()
                        .gap(px(2.0))
                        .flex_shrink_0()
                        .opacity(0.0)
                        .group_hover(
                            SharedString::from(format!("region-row-{for_remove}")),
                            |style| style.opacity(1.0),
                        )
                        .child(icon_button(
                            SharedString::from(format!("region-rename-{for_remove}")),
                            IconName::Pencil,
                            "Rename",
                            cx.listener(move |this, _, window, cx| {
                                let (id, name) = &for_rename;
                                this.start_rename(id.clone(), name, window, cx);
                                cx.stop_propagation();
                            }),
                        ))
                        .child(icon_button(
                            SharedString::from(format!("region-remove-{for_remove}")),
                            IconName::X,
                            "Remove region (keeps nodes)",
                            cx.listener(move |this, _, _, cx| {
                                this.remove(&for_remove, cx);
                                cx.stop_propagation();
                            }),
                        )),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.renaming.is_none() {
                    this.choose(&id, window, cx);
                }
            }))
            .into_any_element()
    }

    /// The reference's trailing row: what is *not* in a region, so the page's blind spot is
    /// visible from the same list its regions are — with the button that asks the model to
    /// clear it.
    fn ungrouped_row(ungrouped: (usize, Option<&Ai>), cx: &App) -> impl IntoElement {
        let (count, ai) = ungrouped;
        let theme = cx.peek_theme();
        div()
            .id("region-row-ungrouped")
            .test_support()
            .h_flex()
            .gap(px(9.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(theme.node_border)
            .text_xs()
            .text_color(theme.fg_muted)
            .child(
                div()
                    .size(DOT)
                    .flex_shrink_0()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.fg_subtle),
            )
            .child(div().flex_1().child("Ungrouped"))
            .child(
                div()
                    .text_size(px(10.5))
                    .font_family("Monaspace Krypton")
                    .text_color(theme.fg_subtle)
                    .child(count.to_string()),
            )
            .children(ai.filter(|ai| ai.can_group_ungrouped).map(|ai| {
                sparkle(
                    "regions-group-ungrouped-ai",
                    "Group ungrouped with AI (slots into existing or creates new)",
                    (Grouping::Ungrouped, ai),
                    cx,
                )
            }))
    }
}

/// One AI button: a sparkle that dispatches its command, or a spinner while it is running.
///
/// Nothing is dispatched while either grouping is in flight. A second ask would race the first
/// over the same regions, and the model is usually the slowest thing in the app.
fn sparkle(
    id: &'static str,
    label: &'static str,
    grouping: (Grouping, &Ai),
    cx: &App,
) -> AnyElement {
    let (grouping, ai) = grouping;
    let theme = cx.peek_theme();
    let button = div()
        .id(id)
        .test_support()
        .aria_label(label)
        .size(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(4.0))
        .text_color(theme.fg_subtle);

    if ai.running == Some(grouping) {
        return button
            .child(Spinner::new().xsmall().color(theme.accent))
            .into_any_element();
    }
    if ai.running.is_some() {
        return button.opacity(0.4).child(icon()).into_any_element();
    }

    let action: Box<dyn Action> = match grouping {
        Grouping::Ungrouped => Box::new(actions::region::GroupWithAi),
        Grouping::All => Box::new(actions::region::RegroupAllWithAi),
    };
    let focus = ai.focus.clone();
    let tooltip: Box<dyn Action> = action.boxed_clone();
    button
        .cursor_pointer()
        .tooltip(move |window, cx| {
            Tooltip::new(label)
                .action(tooltip.as_ref(), Some(crate::commands::CANVAS))
                .build(window, cx)
        })
        .child(icon())
        .on_click(move |_, window, cx| {
            focus.dispatch_action(&*action, window, cx);
            cx.stop_propagation();
        })
        .into_any_element()
}

fn icon() -> Icon {
    Icon::new(IconName::Sparkles).size(px(12.0))
}

fn suggested_badge(cx: &App) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .flex_shrink_0()
        .px(px(5.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(theme.accent_line)
        .bg(theme.accent_bg)
        .text_size(px(9.0))
        .font_family("Monaspace Krypton")
        .text_color(theme.accent)
        .child("SUGGESTED")
}

fn icon_button(
    id: SharedString,
    icon: IconName,
    label: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .size(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .cursor_pointer()
        .child(Icon::new(icon).size(px(12.0)))
        .on_click(on_click)
}

/// A full-window sibling that swallows the press that dismisses the panel. It paints nothing;
/// `occlude` is a hitbox property, so a transparent layer still blocks.
fn scrim(cx: &mut Context<RegionsPanel>) -> impl IntoElement {
    div()
        .id("regions-panel-scrim")
        .test_support()
        .absolute()
        .inset_0()
        .occlude()
        .bg(transparent_black())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| this.close(window, cx)),
        )
}
