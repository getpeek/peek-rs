//! The version-history timeline — the port of `HistoryTimeline.tsx`, `VersionCard.tsx` and
//! `useHistoryPanel.ts`'s selection and keys.
//!
//! A panel pinned to the bottom of the canvas: one dot per checkpoint on a track you drag or
//! scroll, a card over the selected one, and a toast after a restore. The panel only chooses;
//! the canvas shows the chosen version and performs the restore, told through [`PanelEvent`]
//! so neither ever updates the other from inside its own listener.

use std::time::Duration;

use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, Sizable, StyledExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, BoxShadow, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, ScrollWheelEvent,
    SharedString, Subscription, Task, Window, div, point, px,
};
use peek_canvas::timeline::{TRACK_PITCH, Track};
use peek_document::history::{ChangeSummary, HistoryEntry, format};
use peek_document::{CheckpointId, PageId};
use peek_theme::ActivePeekTheme;

use super::store::VersionHistory;

const TOAST: Duration = Duration::from_millis(2400);
/// `.history-panel`, in the same unscaled pixels as the rest of the canvas chrome.
pub(super) const INSET: Pixels = px(16.0);
pub(super) const HEIGHT: Pixels = px(208.0);
const CARD_WIDTH: Pixels = px(300.0);
/// `.history-ring-chip` and `.history-toast` both sit this far from the top.
pub(super) const CHIP_TOP: Pixels = px(46.0);
const WHEEL_LINE: Pixels = px(20.0);

/// What the canvas has to act on. The panel never reaches into the canvas itself: it is
/// opened from inside a canvas listener, where that entity is leased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PanelEvent {
    /// Show this version, or the live page for `None`.
    Preview(Option<CheckpointId>),
    Restore(CheckpointId),
    Closed,
}

/// One dot's worth of an entry, copied out so rendering can hold `&mut Context` while it
/// builds listeners.
struct Dot {
    id: CheckpointId,
    seq: u32,
    taken_at: i64,
    label: Option<String>,
    summary: ChangeSummary,
}

impl Dot {
    fn of(entry: &HistoryEntry) -> Self {
        Self {
            id: entry.id.clone(),
            seq: entry.seq,
            taken_at: entry.taken_at,
            label: entry.label.clone(),
            summary: entry.summary,
        }
    }

    /// `dotSize`: a busier checkpoint is a bigger dot, up to twelve changes.
    fn size(&self) -> Pixels {
        let changes = self.summary.change_count().min(12);
        px(8.0 + f32::from(u8::try_from(changes).unwrap_or(12)) * 0.8)
    }
}

pub(crate) struct HistoryPanel {
    history: Option<Entity<VersionHistory>>,
    page: Option<PageId>,
    open: bool,
    selected: Option<CheckpointId>,
    track: Track,
    /// Where a drag on the track started: the pointer's x and the offset it moved from.
    pan: Option<(f32, f32)>,
    toast: Option<(SharedString, Task<()>)>,
    focus_handle: FocusHandle,
    /// The canvas, on every path that opens the panel. Kept rather than asked for on the way
    /// out, for the reason `wayfinding::menu` gives.
    restore_focus: Option<FocusHandle>,
    history_changes: Option<Subscription>,
}

impl EventEmitter<PanelEvent> for HistoryPanel {}

impl std::fmt::Debug for HistoryPanel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryPanel")
            .field("open", &self.open)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl HistoryPanel {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            history: None,
            page: None,
            open: false,
            selected: None,
            track: Track::default(),
            pan: None,
            toast: None,
            focus_handle: cx.focus_handle(),
            restore_focus: None,
            history_changes: None,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Opens on `page`, capturing it first so the timeline always ends on a dot that *is* now.
    pub(crate) fn open(
        &mut self,
        history: Entity<VersionHistory>,
        page: PageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = true;
        self.restore_focus = window.focused(cx);
        self.history_changes =
            Some(cx.observe(&history, |this, _, cx| this.on_history_changed(cx)));
        self.history = Some(history);
        self.track = Track::default();
        self.follow_page(page, cx);
        window.focus(&self.focus_handle, cx);
    }

    /// The page changed under an open panel: its timeline replaces the old one, starting from
    /// its present.
    pub(crate) fn follow_page(&mut self, page: PageId, cx: &mut Context<Self>) {
        self.page = Some(page);
        self.selected = None;
        self.select_present(cx);
        cx.notify();
    }

    pub(crate) fn page(&self) -> Option<&PageId> {
        self.page.as_ref()
    }

    /// Checkpoints arriving — the log finishing its load, or a capture — follow the present,
    /// and a panel opened before the log arrived gets its first selection now.
    fn on_history_changed(&mut self, cx: &mut Context<Self>) {
        if self.open && self.selected.is_none() {
            self.select_present(cx);
        }
        cx.notify();
    }

    fn select_present(&mut self, cx: &mut Context<Self>) {
        let (Some(history), Some(page)) = (self.history.clone(), self.page.clone()) else {
            return;
        };
        self.selected = history.update(cx, |history, cx| history.capture(&page, None, cx));
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.selected = None;
        self.pan = None;
        self.history_changes = None;
        if let Some(handle) = self.restore_focus.take() {
            window.focus(&handle, cx);
        }
        cx.emit(PanelEvent::Closed);
        cx.notify();
    }

    /// The canvas restored a version and recorded the result as `entry`.
    pub(crate) fn restored(
        &mut self,
        entry: Option<CheckpointId>,
        seq: u32,
        cx: &mut Context<Self>,
    ) {
        self.selected = entry;
        let message = SharedString::from(format!("Restored Version {seq}"));
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        });
        self.toast = Some((message, expiry));
        cx.notify();
    }

    fn dots(&self, cx: &App) -> Vec<Dot> {
        let (Some(history), Some(page)) = (&self.history, &self.page) else {
            return Vec::new();
        };
        history.read(cx).entries(page).iter().map(Dot::of).collect()
    }

    fn present(dots: &[Dot]) -> Option<&CheckpointId> {
        dots.last().map(|dot| &dot.id)
    }

    fn select(&mut self, entry: CheckpointId, cx: &mut Context<Self>) {
        let dots = self.dots(cx);
        let Some(index) = dots.iter().position(|dot| dot.id == entry) else {
            return;
        };
        let is_present = Self::present(&dots) == Some(&entry);
        self.track = self.track.revealing(index);
        self.selected = Some(entry.clone());
        cx.emit(PanelEvent::Preview((!is_present).then_some(entry)));
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let dots = self.dots(cx);
        let Some(index) = self.selected_index(&dots) else {
            return;
        };
        let next = index
            .checked_add_signed(delta)
            .and_then(|next| dots.get(next));
        if let Some(next) = next {
            self.select(next.id.clone(), cx);
        }
    }

    fn selected_index(&self, dots: &[Dot]) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        dots.iter().position(|dot| &dot.id == selected)
    }

    fn restore(&mut self, cx: &mut Context<Self>) {
        let dots = self.dots(cx);
        let Some(selected) = self.selected.clone() else {
            return;
        };
        if Self::present(&dots) == Some(&selected) {
            return;
        }
        cx.emit(PanelEvent::Restore(selected));
    }

    /// `closeCard`: back to the live page with nothing selected.
    fn close_card(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        cx.emit(PanelEvent::Preview(None));
        cx.notify();
    }

    /// Escape steps back to the present first, so ←/→ keep working; a second one closes.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dots = self.dots(cx);
        let present = Self::present(&dots).cloned();
        match present {
            Some(present) if self.selected.as_ref().is_some_and(|id| id != &present) => {
                self.select(present, cx);
            }
            _ => self.close(window, cx),
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => self.escape(window, cx),
            "left" => self.step(-1, cx),
            "right" => self.step(1, cx),
            "enter" => self.restore(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn start_pan(&mut self, event: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {
        self.pan = Some((event.position.x.into(), self.track.offset));
    }

    fn pan_to(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((start, from)) = self.pan else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.pan = None;
            return;
        }
        let moved = f32::from(event.position.x) - start;
        self.track.offset = self.track.clamp(from - moved);
        cx.notify();
    }

    fn end_pan(&mut self, _: &gpui_kit::MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.pan = None;
    }

    /// Whichever axis moved more, as `onWheel` does. gpui's deltas move the content, the
    /// DOM's move the viewport, hence the sign.
    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(WHEEL_LINE);
        let (x, y) = (f32::from(delta.x), f32::from(delta.y));
        let delta = if x.abs() > y.abs() { x } else { y };
        if delta == 0.0 {
            return;
        }
        self.track = self.track.panned_by(-delta);
        cx.notify();
        cx.stop_propagation();
    }

    /// Re-anchors on the present whenever the track changes length or width, as the
    /// reference's effect on `count` and `viewportWidth` does.
    fn fit_track(&mut self, count: usize, viewport: f32) {
        if self.track.count == count && (self.track.viewport - viewport).abs() < f32::EPSILON {
            return;
        }
        self.track = Track {
            count,
            viewport,
            offset: self.track.offset,
        }
        .at_present();
    }
}

impl Render for HistoryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Closed, it still renders an empty layer so it repaints on its own `notify`.
        let layer = div().id("history-layer").absolute().inset_0();
        if !self.open {
            return layer.invisible();
        }
        let dots = self.dots(cx);
        let viewport = f32::from(window.viewport_size().width) - f32::from(INSET) * 2.0;
        self.fit_track(dots.len(), viewport);
        let selected = self.selected_index(&dots);
        let toast = self.toast.as_ref().map(|(message, _)| message.clone());
        layer
            .child(scrim(cx))
            .child(self.panel(&dots, selected, cx))
            .children(toast.map(|message| toast_chip(message, cx)))
    }
}

impl HistoryPanel {
    fn panel(
        &self,
        dots: &[Dot],
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        div()
            .id("history-panel")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context("HistoryPanel")
            .on_key_down(cx.listener(Self::on_key_down))
            .absolute()
            .left(INSET)
            .right(INSET)
            .bottom(INSET)
            .h(HEIGHT)
            .occlude()
            .v_flex()
            .bg(theme.node_bg)
            .border_1()
            .border_color(theme.node_border)
            .rounded(theme.radius_card)
            .when_some(theme.chrome_shadow, |this, (offset_y, blur, color)| {
                this.shadow(vec![BoxShadow {
                    color,
                    offset: point(px(0.0), offset_y),
                    blur_radius: blur,
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .child(Self::head(cx))
            .child(self.viewport(dots, selected, cx))
            .children(selected.map(|index| self.card(dots, index, cx)))
    }

    fn head(cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        div()
            .h_flex()
            .items_center()
            .gap(px(10.0))
            .px(px(18.0))
            .pt(px(12.0))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg)
                    .child("Version History"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(5.0))
                    .text_xs()
                    .text_color(theme.fg_muted)
                    .child(Icon::new(IconName::Hand).size(px(13.0)))
                    .child("Drag to scroll · click a checkpoint"),
            )
            .child(
                Button::new("history-close")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::X))
                    .tooltip("Exit history (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
    }

    fn viewport(
        &self,
        dots: &[Dot],
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let offset = self.track.offset;
        let at = |index: usize| px(Track::dot_x(index) - offset);
        let fill = selected.or(dots.len().checked_sub(1));
        let present = dots.len().checked_sub(1);

        let mut track = div().absolute().inset_0();
        for (index, dot) in dots.iter().enumerate() {
            let new_day = index == 0 || format::is_new_day(dots[index - 1].taken_at, dot.taken_at);
            if !new_day {
                continue;
            }
            if index > 0 {
                track = track.child(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .bottom(px(8.0))
                        .left(px(Track::dot_x(index) - offset - TRACK_PITCH / 2.0))
                        .border_l_1()
                        .border_dashed()
                        .border_color(theme.node_border),
                );
            }
            track = track.child(
                div()
                    .absolute()
                    .bottom(px(6.0))
                    .left(at(index))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_muted)
                    .whitespace_nowrap()
                    .child(format::day(dot.taken_at)),
            );
        }
        track = track
            .child(
                div()
                    .absolute()
                    .top_1_2()
                    .left(px(-offset))
                    .w(px(self.track.width()))
                    .h(px(2.0))
                    .bg(theme.node_border),
            )
            .children(fill.map(|index| {
                div()
                    .absolute()
                    .top_1_2()
                    .left(px(-offset))
                    .w(px(Track::dot_x(index)))
                    .h(px(2.0))
                    .rounded(px(2.0))
                    .bg(theme.accent_line)
            }));
        for (index, dot) in dots.iter().enumerate() {
            let is_selected = selected == Some(index);
            if let Some(label) = &dot.label {
                track = track.child(Self::tag(dot, label, (at(index), is_selected), cx));
            }
            track = track.child(Self::dot(
                dot,
                (at(index), is_selected, present == Some(index)),
                cx,
            ));
        }

        div()
            .id("history-track")
            .test_support()
            .relative()
            .flex_1()
            .overflow_hidden()
            .cursor_grab()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start_pan))
            .on_mouse_move(cx.listener(Self::pan_to))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_pan))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_pan))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(track)
    }

    /// One checkpoint. `state` is its x, whether it is selected, and whether it is the present.
    fn dot(dot: &Dot, state: (Pixels, bool, bool), cx: &mut Context<Self>) -> AnyElement {
        let (x, is_selected, is_present) = state;
        let theme = cx.peek_theme().clone();
        let size = if is_selected {
            dot.size() * 1.7
        } else {
            dot.size()
        };
        let fill = if is_selected || is_present {
            theme.fg
        } else if dot.label.is_some() {
            theme.accent
        } else {
            theme.node_border_strong
        };
        let id = dot.id.clone();
        div()
            .id(SharedString::from(format!("history-dot-{}", dot.id)))
            .test_support()
            .aria_label(SharedString::from(format!("Version {}", dot.seq)))
            .aria_selected(is_selected)
            .absolute()
            .top_1_2()
            .left(x - size / 2.0)
            .mt(-size / 2.0)
            .size(size)
            .rounded_full()
            .bg(fill)
            .cursor_pointer()
            .when(is_selected, |this| {
                this.border_2().border_color(theme.accent)
            })
            .when(is_present && !is_selected, |this| {
                this.border_2().border_color(theme.accent_bg)
            })
            .hover({
                let ring = theme.accent_bg;
                move |style| style.border_2().border_color(ring)
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
            .into_any_element()
    }

    /// A labelled checkpoint's tag above its dot, with the stem down to it.
    fn tag(dot: &Dot, label: &str, state: (Pixels, bool), cx: &mut Context<Self>) -> AnyElement {
        let (x, is_selected) = state;
        let theme = cx.peek_theme().clone();
        let id = dot.id.clone();
        div()
            .id(SharedString::from(format!("history-tag-{}", dot.id)))
            .absolute()
            .left(x)
            .top_1_2()
            .mt(px(-40.0))
            .child(
                div()
                    .absolute()
                    .left(px(-0.5))
                    .top(px(16.0))
                    .w(px(1.0))
                    .h(px(24.0))
                    .bg(theme.accent_line),
            )
            .child(
                div()
                    .relative()
                    .left(px(-60.0))
                    .w(px(120.0))
                    .h_flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(7.0))
                            .rounded(theme.radius_pill)
                            .border_1()
                            .border_color(theme.accent_line)
                            .bg(if is_selected {
                                theme.accent
                            } else {
                                theme.node_bg
                            })
                            .text_color(if is_selected { theme.bg } else { theme.fg })
                            .text_size(px(10.0))
                            .font_family("Monaspace Krypton")
                            .whitespace_nowrap()
                            .truncate()
                            .child(label.to_string()),
                    ),
            )
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
            .into_any_element()
    }

    fn card(&self, dots: &[Dot], index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let dot = &dots[index];
        let is_present = index + 1 == dots.len();
        let (centre, lean) = self.track.card(index);

        div()
            .id("history-card")
            .test_support()
            .absolute()
            // Above the panel's top edge, pointing down at the dot.
            .bottom(HEIGHT + px(14.0))
            .left(px(centre) - CARD_WIDTH / 2.0)
            .w(CARD_WIDTH)
            .occlude()
            .v_flex()
            .bg(theme.node_bg)
            .border_1()
            .border_color(theme.node_border_strong)
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
            .child(Self::summary(dot, is_present, cx))
            .child(
                div().px(px(14.0)).pb(px(10.0)).child(
                    Button::new("history-restore")
                        .primary()
                        .small()
                        .w_full()
                        .icon(Icon::new(IconName::RotateCcw))
                        .label(if is_present {
                            "You are here"
                        } else {
                            "Restore"
                        })
                        .disabled(is_present)
                        .on_click(cx.listener(|this, _, _, cx| this.restore(cx))),
                ),
            )
            .child(Self::pager(index, dots.len(), cx))
            .child(
                // gpui draws no rotated square, so the arrow is a stem from the card's lower
                // edge, leaning to keep pointing at the dot when the card is held at an edge.
                div()
                    .absolute()
                    .top_full()
                    .left(CARD_WIDTH / 2.0 + px(lean) - px(1.0))
                    .w(px(2.0))
                    .h(px(14.0))
                    .bg(theme.node_border_strong),
            )
    }

    fn summary(dot: &Dot, is_present: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let description = dot.label.clone().unwrap_or_else(|| dot.summary.describe());
        let meta = if is_present {
            div().text_color(theme.green).child("Current version")
        } else {
            div()
                .text_color(theme.fg_muted)
                .child(format!("{} changes", dot.summary.change_count()))
        };
        div()
            .v_flex()
            .gap(px(4.0))
            .px(px(14.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg)
                            .child(format!("Version {}", dot.seq)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .font_family("Monaspace Krypton")
                            .text_color(theme.fg_muted)
                            .child(format::stamp(dot.taken_at)),
                    )
                    .child(
                        Button::new("history-card-close")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::X))
                            .tooltip("Close")
                            .on_click(cx.listener(|this, _, _, cx| this.close_card(cx))),
                    ),
            )
            .child(div().text_xs().text_color(theme.fg).child(description))
            .child(meta.text_xs())
    }

    /// Back · i / n · Next.
    fn pager(index: usize, count: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.peek_theme().clone();
        let is_present = index + 1 == count;
        div()
            .h_flex()
            .items_center()
            .px(px(8.0))
            .py(px(4.0))
            .border_t_1()
            .border_color(theme.node_border)
            .bg(theme.node_bg_2)
            .child(
                Button::new("history-back")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::ChevronLeft))
                    .label("Back")
                    .disabled(index == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .h_flex()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.fg_muted)
                    .child(format!("{} / {}", index + 1, count)),
            )
            .child(
                Button::new("history-next")
                    .ghost()
                    .xsmall()
                    .label("Next")
                    .icon(Icon::new(IconName::ChevronRight))
                    .disabled(is_present)
                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
            )
    }
}

/// `.history-scrim`: a veil of the canvas background rising from the bottom, so the panel
/// reads against the board. Pointer-transparent, as the reference's is.
fn scrim(cx: &App) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(gpui_kit::relative(0.38))
        .bg(gpui_kit::linear_gradient(
            0.0,
            gpui_kit::linear_color_stop(theme.bg.opacity(0.82), 0.0),
            gpui_kit::linear_color_stop(theme.bg.opacity(0.0), 1.0),
        ))
}

fn toast_chip(message: SharedString, cx: &App) -> impl IntoElement {
    let theme = cx.peek_theme();
    div()
        .id("history-toast")
        .test_support()
        .absolute()
        .top(CHIP_TOP)
        .left_0()
        .right_0()
        .h_flex()
        .justify_center()
        .child(
            div()
                .h_flex()
                .items_center()
                .gap(px(8.0))
                .px(px(14.0))
                .py(px(8.0))
                .rounded(theme.radius_card)
                .border_1()
                .border_color(theme.accent_line)
                .bg(theme.node_bg)
                .text_xs()
                .text_color(theme.fg)
                .child(div().size(px(7.0)).rounded_full().bg(theme.green))
                .child(message),
        )
}
