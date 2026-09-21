//! The canvas view: owns the camera, flights and the gesture state machine, and renders the
//! `CanvasElement` that places node shells at camera-derived screen positions.

pub(crate) mod context_menu;
mod convert;
mod dispatch;
mod edges;
mod element;
mod frame_stats;
mod grid;
mod hud;
pub(crate) mod json_editor;
mod jump;
mod page_search;
mod toolbar;
mod tools;
mod wayfinding;

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use gpui_kit::TestSupportExt;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Context, CursorStyle, Entity, FocusHandle, Hsla, KeyDownEvent, KeyUpEvent,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels,
    ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task, TouchPhase, Window, div,
};
use peek_canvas::camera::FitOptions;
use peek_canvas::direction::{self, Direction};
use peek_canvas::edge::curve_between;
use peek_canvas::flight::durations;
use peek_canvas::gesture::{self, Effect, GestureConfig, Interaction};
use peek_canvas::hit::{Corner, NodeRegion};
use peek_canvas::jump::{JumpMode, Pressed};
use peek_canvas::layout::bsp;
use peek_canvas::render_scale;
use peek_canvas::{Camera, CameraFlight, Detail, Document, Layout, Point, Rect, Scope, Size};
use peek_document::{Edge, NodeId, NodeKind, NodeType, PageId};
use peek_theme::{ActivePeekTheme, EdgeState};

use crate::commands::{self, actions};
use crate::node::state::NodeStates;
use crate::node::{self, NodeShell};
use crate::title_bar::close_page;
use convert::{to_pixel_bounds, to_pixel_point};
use edges::EdgeItem;
use element::{CanvasElement, NodeItem, Overlay};
use frame_stats::{FrameStats, Phase, Reading};

/// Screen-space slack around the viewport so nodes at the edge are built before they scroll in.
const CULL_MARGIN_PX: f64 = 64.0;
/// React Flow's zoom button step.
const ZOOM_STEP: f64 = 1.2;
/// Mouse wheels have no gesture phases; the viewport is committed after this quiet period.
const WHEEL_COMMIT_DELAY: Duration = Duration::from_millis(140);

struct ActiveFlight {
    flight: CameraFlight,
    started: Instant,
}

/// A force layout in progress. Ticked from `render` on the wall clock exactly as a flight is;
/// the run itself lives in [`dispatch::layout`].
struct OrganizeRun {
    layout: Layout,
    last_tick: Instant,
}

pub(crate) struct CanvasView {
    document: Entity<Document>,
    camera: Camera,
    flight: Option<ActiveFlight>,
    interaction: Interaction,
    space_held: bool,
    camera_locked: bool,
    focus_handle: FocusHandle,
    /// Window-space bounds of the pane, written back by the element every prepaint.
    pane_bounds: Option<Bounds<Pixels>>,
    wheel_generation: u64,
    /// What the pointer is over while no gesture is running, so the cursor can show the
    /// affordance before the press.
    hovered: Hover,
    /// Retained per-node view state (editors, scroll positions) for the kinds that need it.
    node_states: NodeStates,
    /// `View::ToggleUi` hides every chrome surface, not only the title bar.
    chrome_visible: bool,
    /// Jump mode: the labelled targets and what has been typed, or `None` when it is off.
    jump: Option<JumpMode>,
    /// The page-search panel, or `None` when it is closed.
    page_search: Option<page_search::PageSearch>,
    /// The right-click menu a node raised, or `None` when none is open.
    context_menu: Option<context_menu::MenuState>,
    /// The JSON editor raised by a result cell, or `None`. Chrome rather than content, so it
    /// lives here beside the context menu — `canvas/json_editor.rs` says why.
    json_editor: Option<json_editor::JsonEditorState>,
    /// The node a running placement drag created, which the rest of the drag resizes. The
    /// reducer tracks the gesture; the id lives here because the document mints it.
    placement: Option<NodeId>,
    /// Drops jump mode and the space-pan when focus leaves. Both are latched state that only
    /// a key *release* would otherwise clear, and that release never arrives once a dialog or
    /// another view has taken focus.
    _focus_out: Subscription,
    /// The force layout `View::Organize` and `View::Schema` run, or `None` when nothing is
    /// being arranged.
    organize: Option<OrganizeRun>,
    /// Frame timings, inert unless `PEEK_FRAME_STATS=1` or `--fps`.
    frame_stats: FrameStats,
    /// One deferred repaint, so the frame-rate readout can settle to `idle` after the last
    /// frame of a gesture. Re-armed from `render`, which drops the pending one, so only the
    /// last frame of a burst ever fires. `None` without `--fps`: nothing else needs it.
    fps_settle: Option<Task<()>>,
    /// Whether node bodies are being built at this zoom. Retained because the thresholds
    /// overlap: the tier inside the band is whatever the last frame settled on.
    detail: Detail,
    /// Regions on screen: the beacon drag, the flash ring and the peekers' quiet period.
    wayfinding: wayfinding::Wayfinding,
    /// The regions picker above the zoom cluster. Its own entity, like the pages picker, so a
    /// rename field can own focus without the canvas re-rendering behind every keystroke.
    regions: Entity<wayfinding::menu::RegionsPanel>,
    /// The Keep / Rename / Dismiss cards over suggested regions. Its own entity for the same
    /// reason the picker is: it owns a rename field, and a field needs focus.
    suggestions: Entity<wayfinding::card::SuggestionCards>,
    /// The AI grouping in flight, or `None`. Held here rather than in the picker because the
    /// palette can start one with the picker closed, and dropping it cancels the request.
    grouping: Option<dispatch::regions::GroupingRun>,
}

impl std::fmt::Debug for CanvasView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanvasView")
            .field("camera", &self.camera)
            .field("interaction", &self.interaction)
            .field("camera_locked", &self.camera_locked)
            .finish_non_exhaustive()
    }
}

impl CanvasView {
    pub(crate) fn new(
        document: Entity<Document>,
        focus_handle: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let camera = Camera::from_viewport(document.read(cx).viewport());
        cx.observe_in(&document, window, |this, document, window, cx| {
            cx.notify();
            this.frame_requested(&document, window, cx);
        })
        .detach();
        let focus_out = cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
            this.space_held = false;
            if this.jump.take().is_some() {
                cx.notify();
            }
        });
        let canvas = cx.entity().downgrade();
        let regions = cx.new(|cx| wayfinding::menu::RegionsPanel::new(canvas.clone(), window, cx));
        let suggestions = cx.new(|cx| wayfinding::card::SuggestionCards::new(canvas, window, cx));
        Self {
            node_states: NodeStates::new(document.clone(), cx.entity().downgrade()),
            document,
            camera,
            flight: None,
            interaction: Interaction::Idle,
            space_held: false,
            camera_locked: false,
            focus_handle,
            pane_bounds: None,
            wheel_generation: 0,
            hovered: Hover::default(),
            chrome_visible: true,
            jump: None,
            page_search: None,
            context_menu: None,
            json_editor: None,
            placement: None,
            _focus_out: focus_out,
            organize: None,
            frame_stats: FrameStats::new(crate::settings::Settings::fps(cx)),
            fps_settle: None,
            detail: Detail::Full,
            wayfinding: wayfinding::Wayfinding::default(),
            regions,
            suggestions,
            grouping: None,
        }
    }

    /// Lets [`CanvasElement`] report its own phases; the element holds the view, not the stats.
    pub(crate) fn record_frame_phase(&mut self, phase: Phase, elapsed: Duration) {
        self.frame_stats.record(phase, elapsed);
    }

    pub(crate) fn frame_stats_enabled(&self) -> bool {
        self.frame_stats.enabled()
    }

    /// The frame rate the HUD draws, or `None` when `--fps` was not given or the canvas is idle.
    pub(super) fn fps_reading(&self) -> Option<Reading> {
        self.frame_stats
            .fps_enabled()
            .then(|| self.frame_stats.reading(Instant::now()))
            .flatten()
    }

    pub(super) fn fps_enabled(&self) -> bool {
        self.frame_stats.fps_enabled()
    }

    /// Schedules the one repaint that lets the readout fall back to `idle`.
    ///
    /// A passive counter only updates on a frame that was going to happen anyway, so without
    /// this the pill would sit frozen on the last number a gesture produced. Re-arming drops
    /// the pending task, so a burst of frames still costs exactly one trailing repaint.
    fn arm_fps_settle(&mut self, cx: &mut Context<Self>) {
        if !self.frame_stats.fps_enabled() {
            return;
        }
        self.fps_settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(frame_stats::IDLE_AFTER)
                .await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    pub(crate) fn camera(&self) -> Camera {
        self.camera
    }

    /// Where the camera will settle: the flight's destination while one is running.
    pub(crate) fn camera_target(&self) -> Camera {
        self.flight
            .as_ref()
            .map_or(self.camera, |active| active.flight.to)
    }

    pub(crate) fn document(&self) -> &Entity<Document> {
        &self.document
    }

    pub(crate) fn is_camera_locked(&self) -> bool {
        self.camera_locked
    }

    /// Whether the regions picker is up, so the cluster's trigger can show it.
    pub(crate) fn regions_open(&self, cx: &App) -> bool {
        self.regions.read(cx).is_open()
    }

    /// The table behind a result node, for tests.
    ///
    /// `DataTable` registers no id of its own, so a test cannot reach it through `window.find`;
    /// `TableState`'s `dump_range` and `visible_range` are the observables, and this is the only
    /// way to them. See `docs/testing.md`, "Asserting on a component you cannot name".
    /// The view behind a result node.
    ///
    /// The scoped `Result::*` commands are handled here rather than on the node — the palette
    /// dispatches through the canvas focus handle, which is an ancestor of node elements — so the
    /// canvas has to be able to reach the table it is acting on.
    pub(crate) fn result_inner(
        &self,
        node: &peek_document::NodeId,
    ) -> Option<gpui_kit::Entity<node::result::ResultTable>> {
        match self.node_states.peek(node)? {
            node::state::NodeState::Result(state) => Some(state.inner()),
            _ => None,
        }
    }

    /// The view behind an agent node, for tests.
    #[cfg(test)]
    pub(crate) fn agent_view(
        &self,
        node: &peek_document::NodeId,
        _cx: &App,
    ) -> Option<gpui_kit::Entity<node::agent::AgentView>> {
        match self.node_states.peek(node)? {
            node::state::NodeState::Agent(state) => Some(state.view().clone()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn result_table(
        &self,
        node: &peek_document::NodeId,
        cx: &App,
    ) -> Option<
        gpui_kit::Entity<gpui_kit::component::table::TableState<node::result::ResultDelegate>>,
    > {
        match self.node_states.peek(node)? {
            node::state::NodeState::Result(state) => Some(state.table(cx)),
            _ => None,
        }
    }

    /// Takes focus back when the element that held it has gone.
    ///
    /// A node whose body owns a focus handle — the SQL editor, the results table — takes that
    /// handle with it when the node is deleted, and gpui is then focused on nothing. Every
    /// binding on the canvas is dispatched through the focus path, so that silently kills
    /// `cmd-z`, `backspace` and the rest until something else is clicked. `docs/canvas.md`
    /// describes the trap; this is the one place that recovers from it, so it covers deletion by
    /// any route — the Delete key, undo, MCP, or a page switch.
    /// The detail tier for this frame.
    ///
    /// Held at [`Detail::Full`] unless the run was launched with `--performance`: dropping the
    /// body of a distant node is a trade, and the default side of it is the honest canvas.
    ///
    /// Held there too whenever focus is somewhere other than the canvas itself, which
    /// means an editor owns it. Dropping that editor's element mid-edit would kill its focus
    /// handle and put the caret back to wherever it lands on the way in; `reclaim_focus` keeps
    /// that from breaking the key bindings, but it cannot put the caret back. It costs nothing
    /// in practice — a camera far enough out to reduce a node is too far out to read one, let
    /// alone type into it.
    fn resolved_detail(&self, window: &Window, cx: &App) -> Detail {
        if !crate::settings::Settings::performance(cx) {
            return Detail::Full;
        }
        if window
            .focused(cx)
            .is_some_and(|focused| focused != self.focus_handle)
        {
            return Detail::Full;
        }
        peek_canvas::lod::detail(self.camera.zoom, self.detail)
    }

    /// Hands focus back to the canvas. Chrome that owned a field calls this on the way out: a
    /// window left focused on an element that no longer renders silently kills every canvas
    /// binding, `cmd-z` included.
    pub(crate) fn take_focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    fn reclaim_focus(&self, window: &mut Window, cx: &mut App) {
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
    }

    pub(crate) fn scope(&self, cx: &App) -> Scope {
        let settings = crate::settings::Settings::get(cx);
        Scope {
            camera_locked: self.camera_locked,
            chrome_hidden: !self.chrome_visible,
            connected: crate::database::Database::is_connected(cx),
            settings: peek_canvas::SettingsScope {
                pages_as_list: settings.ui.pages.show_as == peek_config::PageDisplay::List,
                palette_button_hidden: settings.ui.titlebar.command_palette_button
                    == peek_config::Visibility::Hide,
                regions_enabled: settings.canvas.enable_regions,
            },
            ai: peek_canvas::AiScope {
                local_model: settings.ai.ollama.is_some(),
                labels_queries: settings.ai.automatically_label_queries,
            },
            ..self.document.read(cx).scope()
        }
    }

    pub(crate) fn set_chrome_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.chrome_visible == visible {
            return;
        }
        self.chrome_visible = visible;
        cx.notify();
    }

    /// The place tool currently armed, if any — what lights the toolbar's active button.
    fn armed_tool(&self) -> Option<NodeType> {
        self.interaction.armed_tool()
    }

    // ---- geometry helpers ----------------------------------------------------------------

    fn pane_size(&self, window: &Window) -> Size {
        let size = self
            .pane_bounds
            .map_or_else(|| window.viewport_size(), |bounds| bounds.size);
        convert::from_pixel_size(size)
    }

    /// A camera computed against [`Self::framing_pane`] places content at the pane's top; push
    /// it down past the chrome.
    fn below_chrome(camera: Camera, top: f64) -> Camera {
        Camera {
            pan: Point::new(camera.pan.x, camera.pan.y + top),
            ..camera
        }
    }

    fn pane_origin(&self) -> Point {
        self.pane_bounds.map_or_else(Point::default, |bounds| {
            convert::from_pixel_point(bounds.origin)
        })
    }

    /// Height of the chrome overlaying the top of the pane. The canvas fills the window and the
    /// title bar floats above it, so anything that *frames* content has to avoid the covered
    /// strip or half of what it framed lands behind the bar. Culling and hit-testing still use
    /// the real pane.
    fn chrome_inset(&self) -> f64 {
        if self.chrome_visible {
            f64::from(crate::title_bar::HEIGHT)
        } else {
            0.0
        }
    }

    /// The pane minus that strip, and how far down it starts.
    fn framing_pane(&self, window: &Window) -> (Size, f64) {
        let pane = self.pane_size(window);
        let top = self.chrome_inset().min(pane.height);
        (Size::new(pane.width, pane.height - top), top)
    }

    fn pane_center(&self, window: &Window) -> Point {
        let (size, top) = self.framing_pane(window);
        Point::new(size.width / 2.0, top + size.height / 2.0)
    }

    /// Converts a window-space pointer position into pane space (the camera's screen space).
    /// Raises a right-click menu at `at` (pane coordinates). The caller builds the entries; the
    /// canvas only knows how to draw a list of labelled actions.
    pub(crate) fn open_context_menu(
        &mut self,
        menu: context_menu::MenuState,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = Some(menu);
        cx.notify();
    }

    /// Opens one row's submenu, or closes whichever was open. Notifies only on a real change,
    /// because this runs on every pointer move across the menu.
    pub(crate) fn set_open_submenu(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(menu) = self.context_menu.as_mut() else {
            return;
        };
        if menu.open_submenu == index {
            return;
        }
        menu.open_submenu = index;
        cx.notify();
    }

    pub(crate) fn open_json_editor(
        &mut self,
        editor: json_editor::JsonEditorState,
        cx: &mut Context<Self>,
    ) {
        self.json_editor = Some(editor);
        cx.notify();
    }

    /// Moves the panel to where its anchor cell now is.
    ///
    /// The cell reports on every frame it is drawn, so this notifies only on a real change —
    /// a repaint per report would request the next frame that produces the next report.
    pub(crate) fn move_json_editor(&mut self, anchor: Bounds<Pixels>, cx: &mut Context<Self>) {
        let Some(editor) = self.json_editor.as_mut() else {
            return;
        };
        if editor.anchor == anchor {
            return;
        }
        editor.anchor = anchor;
        cx.notify();
    }

    /// Closes the panel and clears the edit behind it, so the cell stops being an anchor.
    pub(crate) fn close_json_editor(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.json_editor.take() else {
            return false;
        };
        if let Some(table) = editor.table.upgrade() {
            table.update(cx, crate::node::result::ResultTable::cancel_edit);
        }
        cx.notify();
        true
    }

    pub(crate) fn close_context_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.context_menu.take().is_some();
        if was_open {
            cx.notify();
        }
        was_open
    }

    /// The pane's size in plain floats, for clamping a menu inside it.
    pub(crate) fn pane_size_for_menu(&self) -> (f32, f32) {
        self.pane_bounds.map_or((0.0, 0.0), |bounds| {
            (bounds.size.width.into(), bounds.size.height.into())
        })
    }

    pub(crate) fn to_pane(&self, window_position: gpui_kit::Point<Pixels>) -> Point {
        convert::from_pixel_point(window_position) - self.pane_origin()
    }

    pub(crate) fn set_pane_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.pane_bounds == Some(bounds) {
            return;
        }
        self.pane_bounds = Some(bounds);
        cx.notify();
    }

    // ---- camera --------------------------------------------------------------------------

    fn fly_to(&mut self, to: Camera, duration: Duration, window: &Window, cx: &mut Context<Self>) {
        log::debug!(
            "peek: flight {:?} -> {:?} over {duration:?}",
            self.camera,
            to
        );
        if cx.reduce_motion() || duration.is_zero() || self.camera.approx_eq(to) {
            self.camera = to;
            self.flight = None;
            self.commit_viewport(cx);
            cx.notify();
            return;
        }
        self.flight = Some(ActiveFlight {
            flight: CameraFlight::new(self.camera, to, duration, self.pane_size(window)),
            started: Instant::now(),
        });
        cx.notify();
    }

    fn tick_flight(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(active) = &self.flight else {
            return;
        };
        let elapsed = active.started.elapsed();
        if active.flight.is_finished(elapsed) {
            self.camera = active.flight.to;
            self.flight = None;
            self.commit_viewport(cx);
            return;
        }
        self.camera = active.flight.sample(active.flight.progress(elapsed));
        self.nudge_peekers(cx);
        window.request_animation_frame();
    }

    /// The only place the viewport is persisted: gesture end, flight end, wheel quiet period.
    fn commit_viewport(&mut self, cx: &mut Context<Self>) {
        let viewport = self.camera.to_viewport();
        self.document.update(cx, |document, cx| {
            document.set_viewport(viewport);
            cx.notify();
        });
    }

    fn schedule_wheel_commit(&mut self, cx: &mut Context<Self>) {
        self.wheel_generation += 1;
        let generation = self.wheel_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WHEEL_COMMIT_DELAY).await;
            this.update(cx, |view, cx| {
                if view.wheel_generation == generation {
                    view.commit_viewport(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- actions -------------------------------------------------------------------------

    fn zoom_in(&mut self, _: &actions::zoom::In, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .camera
            .zoomed_by_about(self.pane_center(window), ZOOM_STEP);
        self.fly_to(target, durations::ZOOM_BUTTON, window, cx);
    }

    fn zoom_out(&mut self, _: &actions::zoom::Out, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .camera
            .zoomed_by_about(self.pane_center(window), 1.0 / ZOOM_STEP);
        self.fly_to(target, durations::ZOOM_BUTTON, window, cx);
    }

    fn reset_zoom(
        &mut self,
        _: &actions::zoom::Reset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.camera.zoomed_about(self.pane_center(window), 1.0);
        self.fly_to(target, durations::RESET_ZOOM, window, cx);
    }

    fn fit_view(
        &mut self,
        _: &actions::zoom::FitView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.document.read(cx).content_bounds() else {
            return;
        };
        let (pane, top) = self.framing_pane(window);
        let target =
            Self::below_chrome(Camera::fit_bounds(bounds, pane, FitOptions::default()), top);
        self.fly_to(target, durations::FIT_VIEW, window, cx);
    }

    /// Flies to the nodes a finished run asked for, `focusCreated` in `executeQueries.ts`.
    ///
    /// The request is drained here rather than acted on where the run placed the nodes: the
    /// camera needs the pane it is framing into, and only the view knows that.
    fn frame_requested(
        &mut self,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested = document.update(cx, |document, _| document.take_framing());
        let Some(bounds) = document.read(cx).bounds_of(&requested) else {
            return;
        };
        let (pane, top) = self.framing_pane(window);
        let target = Self::below_chrome(
            Camera::fit_bounds(bounds, pane, FitOptions::padding(0.2)),
            top,
        );
        self.fly_to(target, durations::FIT_NODES, window, cx);
    }

    /// `fitNodesToView.tsx`: tile the selection across the viewport at 100% zoom.
    ///
    /// The only zoom command that *writes* to the document. It lays the nodes out to fill the
    /// pane rather than flying the camera out to wherever they already are — `bsp::fit_selection`
    /// does the layout, and the camera then simply goes to zoom 1 over the point it was already
    /// looking at, which is the viewport `computeViewportFit` returns.
    ///
    /// The nodes jump and only the camera tweens, as they do in the reference: a resize is a
    /// document mutation, and animating one would be an autosave per frame.
    fn fit_selection(
        &mut self,
        _: &actions::zoom::FitSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let center = self.focus_point(window);
        let (pane, _) = self.framing_pane(window);
        let laid_out = self.document.update(cx, |document, cx| {
            // Seals whatever edit was open, so the fit is its own undo step rather than the tail
            // of the drag that selected the nodes.
            document.checkpoint();
            let laid_out = bsp::fit_selection(document, pane, center);
            if laid_out {
                cx.notify();
            }
            document.checkpoint();
            laid_out
        });
        if !laid_out {
            return;
        }
        self.fly_to(
            self.centred_on(center, 1.0, window),
            durations::FIT_SELECTED,
            window,
            cx,
        );
    }

    // ---- keyboard navigation -------------------------------------------------------------

    /// Where the camera is looking, in world units: the anchor for both jump labels and the
    /// first arrow press.
    fn focus_point(&self, window: &Window) -> Point {
        self.camera.screen_to_world(self.pane_center(window))
    }

    /// Centres `world` without changing the zoom, leaving room for the chrome the same way
    /// `fit_view` does.
    fn centred_on(&self, world: Point, zoom: f64, window: &Window) -> Camera {
        let (pane, top) = self.framing_pane(window);
        Self::below_chrome(Camera::centered_on(world, zoom, pane), top)
    }

    fn go_to_node(
        &mut self,
        _: &actions::page::GoToNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.camera.visible_world_rect(self.pane_size(window));
        let document = self.document.read(cx);
        let Some(jump) = JumpMode::new(document.nodes(), visible, self.focus_point(window)) else {
            return;
        };
        self.jump = Some(jump);
        cx.notify();
    }

    fn exit_jump(&mut self, cx: &mut Context<Self>) {
        if self.jump.take().is_some() {
            cx.notify();
        }
    }

    /// Selects `id` alone and flies to it. `zoom` is what separates the two callers: jump mode
    /// always lands at 1, an arrow key keeps the camera where it is.
    fn go_to(&mut self, id: &NodeId, zoom: f64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(centre) = self
            .document
            .read(cx)
            .node(id)
            .map(|node| node.bounds().center())
        else {
            return;
        };
        let ids = [id.clone()];
        self.document.update(cx, |document, cx| {
            if document.select_only(ids) {
                cx.notify();
            }
        });
        let target = self.centred_on(centre, zoom, window);
        self.fly_to(target, durations::PAN_TO, window, cx);
    }

    fn select_node_left(
        &mut self,
        _: &actions::page::SelectNodeLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_selection(Direction::Left, window, cx);
    }

    fn select_node_right(
        &mut self,
        _: &actions::page::SelectNodeRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_selection(Direction::Right, window, cx);
    }

    fn select_node_up(
        &mut self,
        _: &actions::page::SelectNodeUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_selection(Direction::Up, window, cx);
    }

    fn select_node_down(
        &mut self,
        _: &actions::page::SelectNodeDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_selection(Direction::Down, window, cx);
    }

    /// With nothing selected the first press ignores the direction and anchors on whatever the
    /// camera is looking at; after that it walks the 45° cone.
    fn step_selection(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.document.read(cx);
        let nodes = document.nodes();
        // Document order, not the selection set's id order: the reference steps from React
        // Flow's first selected node, and a `BTreeSet` would pick by id instead.
        let origin = nodes.iter().find(|node| document.is_selected(&node.id));
        let target = match origin {
            Some(origin) => direction::node_in_direction(nodes, origin, direction),
            None => direction::nearest_to(nodes, self.focus_point(window)),
        };
        let Some(id) = target.cloned() else {
            return;
        };
        // The settled zoom, not the live one: arrowing away mid-flight would otherwise freeze
        // the camera at whatever fraction of the previous flight had elapsed.
        self.go_to(&id, self.camera_target().zoom, window, cx);
    }

    /// Enter drops into the selected query's editor. The editor stashes what held focus when
    /// it takes it, so escape hands it straight back to the canvas.
    fn focus_query(
        &mut self,
        _: &actions::query::Focus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.document.read(cx);
        let selected = document.selected();
        if selected.len() != 1 {
            return;
        }
        let Some(node) = selected.iter().next().and_then(|id| document.node(id)) else {
            return;
        };
        if !matches!(node.kind, NodeKind::Query(_)) {
            return;
        }
        let node = node.clone();
        self.focus_query_editor(&node, window, cx);
    }

    /// Hands focus to a query node's SQL editor, creating its retained state if the node has
    /// not rendered yet — which is the case for a query the pointer just placed.
    fn focus_query_editor(
        &mut self,
        node: &peek_document::Node,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(node::state::NodeState::Query(state)) = self.node_states.get(node, window, cx) {
            state.focus_editor(window, cx);
        }
    }

    fn select_all(&mut self, _: &actions::edit::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| {
            if document.select_all() {
                cx.notify();
            }
        });
    }

    fn clear_selection(
        &mut self,
        _: &actions::tool::Select,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Escape stays bound on the bare `Canvas` context, and a bound key never reaches a key
        // listener — so the only place jump mode can see it is here. It cancels the jump and
        // leaves the selection alone, as the reference does.
        if self.jump.is_some() {
            self.exit_jump(cx);
            return;
        }
        // A menu is the most transient surface on the canvas: escape dismisses it before it
        // reaches anything the user might actually lose.
        if self.close_context_menu(cx) {
            return;
        }
        // The JSON editor holds focus while it is up, so escape dispatches out of it to here
        // rather than to the result node — which is why the node's own escape rule cannot see
        // it. Cancelling a draft loses more than a selection does, so it goes before them.
        if self.close_json_editor(cx) {
            return;
        }
        // The picker is a panel over the canvas; escape takes it down before it reaches the
        // selection underneath. Bound actions never reach a key listener, so the panel's own
        // `on_key_down` only sees escape while its rename field has focus — this covers the
        // rest.
        if self.close_regions_picker(window, cx) {
            return;
        }
        self.interaction = Interaction::Idle;
        self.cancel_placement(cx);
        self.document.update(cx, |document, cx| {
            if document.deselect_all() {
                cx.notify();
            }
        });
        cx.notify();
    }

    fn previous_page(
        &mut self,
        _: &actions::page::Previous,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_page(-1, cx);
    }

    // ---- document commands ---------------------------------------------------------------

    fn delete_selection(
        &mut self,
        _: &actions::edit::DeleteSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.document.update(cx, |document, cx| {
            if document.delete_selection() > 0 {
                document.checkpoint();
                cx.notify();
            }
        });
    }

    fn undo(&mut self, _: &actions::history::Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| {
            if document.undo() {
                cx.notify();
            }
        });
    }

    fn redo(&mut self, _: &actions::history::Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| {
            if document.redo() {
                cx.notify();
            }
        });
    }

    fn new_page(&mut self, _: &actions::page::New, _: &mut Window, cx: &mut Context<Self>) {
        self.commit_viewport(cx);
        self.document.update(cx, |document, cx| {
            document.add_page(None, None);
            cx.notify();
        });
        self.adopt_active_page(cx);
    }

    // ---- place tools ---------------------------------------------------------------------

    fn place_text(&mut self, _: &actions::tool::Text, window: &mut Window, cx: &mut Context<Self>) {
        self.arm_tool(NodeType::Text, window, cx);
    }

    fn place_query(
        &mut self,
        _: &actions::tool::Query,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.arm_tool(NodeType::Query, window, cx);
    }

    fn place_agent(
        &mut self,
        _: &actions::tool::Agent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.arm_tool(NodeType::Agent, window, cx);
    }

    fn place_variable(
        &mut self,
        _: &actions::tool::Variable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.arm_tool(NodeType::Variable, window, cx);
    }

    fn place_drawing(
        &mut self,
        _: &actions::tool::Draw,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.arm_tool(NodeType::Draw, window, cx);
    }

    /// Forks the selected agent conversation into a sibling node and flies to it.
    ///
    /// Handled here rather than on the node because it needs the camera, and because the header
    /// button, the palette and the keyboard all have to run this one path.
    fn fork_agent(
        &mut self,
        _: &actions::agent::Fork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.document.read(cx).selected().iter().next().cloned() else {
            return;
        };
        let forked = self.document.update(cx, |document, cx| {
            let forked = document.fork_agent(&source);
            if forked.is_some() {
                document.checkpoint();
                cx.notify();
            }
            forked
        });
        let Some(forked) = forked else {
            return;
        };
        let Some(bounds) = self
            .document
            .read(cx)
            .node(&forked)
            .map(peek_document::Node::bounds)
        else {
            return;
        };
        let target = self.centred_on(bounds.center(), self.camera.zoom, window);
        self.fly_to(target, durations::FIT_SELECTED, window, cx);
    }

    /// Arms place mode: the next click places the node, a drag sizes it, escape cancels.
    fn arm_tool(&mut self, node_type: NodeType, window: &mut Window, cx: &mut Context<Self>) {
        self.reduce(gesture::Input::ArmTool(Some(node_type)), window, cx);
        cx.notify();
    }

    fn next_page(&mut self, _: &actions::page::Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step_page(1, cx);
    }

    /// Closes the active page. A page with no nodes goes immediately — there is nothing to
    /// lose — and anything else confirms first, because deletion is not undoable. The keyboard
    /// path lands here too, so `cmd-w` asks as well.
    fn close_page(
        &mut self,
        _: &actions::page::Close,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (id, name, nodes, closable) = {
            let document = self.document.read(cx);
            let page = document.active_page();
            (
                page.id.clone(),
                SharedString::from(page.name.clone()),
                page.nodes.len(),
                document.page_count() > 1,
            )
        };
        if !closable {
            return;
        }
        if nodes == 0 {
            self.delete_page(&id, cx);
            return;
        }
        close_page::ClosePageConfirm::open(&cx.entity(), &id, name, nodes, window, cx);
    }

    fn step_page(&mut self, offset: isize, cx: &mut Context<Self>) {
        let Some(target) = self.document.read(cx).neighbour_page(offset).cloned() else {
            return;
        };
        self.switch_to_page(&target, cx);
    }

    /// Switches to a page by id — what a tab click does. Clicking one tab is direct
    /// manipulation rather than a keyboard-addressable command, and gpui actions here carry no
    /// payload, so this is a method rather than an action.
    pub(crate) fn switch_to_page(&mut self, id: &PageId, cx: &mut Context<Self>) -> bool {
        // `set_viewport` writes to whichever page is active, so the outgoing camera has to be
        // committed before the switch, not after.
        self.commit_viewport(cx);
        let switched = self.document.update(cx, |document, cx| {
            let switched = document.switch_page(id);
            if switched {
                cx.notify();
            }
            switched
        });
        if switched {
            self.adopt_active_page(cx);
        }
        switched
    }

    /// Moves a page to the slot `to` in the tab strip — what dropping one tab on another does.
    pub(crate) fn reorder_page(&mut self, id: &PageId, to: usize, cx: &mut Context<Self>) -> bool {
        self.document.update(cx, |document, cx| {
            let moved = document.reorder_page(id, to);
            if moved {
                cx.notify();
            }
            moved
        })
    }

    pub(crate) fn rename_page(
        &mut self,
        id: &PageId,
        name: String,
        cx: &mut Context<Self>,
    ) -> bool {
        self.document.update(cx, |document, cx| {
            let renamed = document.rename_page(id, name);
            if renamed {
                cx.notify();
            }
            renamed
        })
    }

    /// Deletes a page without asking. The caller decides whether to confirm first: deletion is
    /// not undoable.
    pub(crate) fn delete_page(&mut self, id: &PageId, cx: &mut Context<Self>) -> bool {
        let deleted = self.document.update(cx, |document, cx| {
            let deleted = document.delete_page(id);
            if deleted {
                cx.notify();
            }
            deleted
        });
        if deleted {
            // Deleting the active page moves the document to its neighbour, so the camera has
            // to follow here too.
            self.adopt_active_page(cx);
        }
        deleted
    }

    /// Drops anything tied to the outgoing page and reloads the camera from the page the
    /// document now calls active. The single place a page change touches the camera.
    fn adopt_active_page(&mut self, cx: &mut Context<Self>) {
        self.flight = None;
        self.interaction = Interaction::Idle;
        self.camera = Camera::from_viewport(self.document.read(cx).viewport());
        cx.notify();
    }

    fn toggle_camera_lock(
        &mut self,
        _: &actions::view::ToggleCameraLock,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.camera_locked = !self.camera_locked;
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.jump.is_some() {
            self.jump_key(event, window, cx);
            return;
        }
        if event.keystroke.key == "space" && !self.space_held {
            self.space_held = true;
            cx.notify();
        }
    }

    /// Jump mode's keyboard, in the reference's order. Only keys with no live binding arrive
    /// here, which is exactly what `CANVAS_JUMPING` arranges: every single-letter canvas
    /// command goes dead while a label is being typed.
    fn jump_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A held letter would spam the prefix; macOS delivers repeats as full key-downs.
        if event.is_held {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        // Any modifier hands the combo back untouched, so `cmd-p` still opens the palette.
        if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
            self.exit_jump(cx);
            return;
        }
        cx.stop_propagation();

        if keystroke.key == "backspace" {
            if let Some(jump) = &mut self.jump {
                jump.backspace();
            }
            cx.notify();
            return;
        }
        // `key`, never `key_char`: macOS normalises it to the ASCII equivalent on other
        // layouts, and leaves `key_char` empty whenever a modifier is down.
        let letter = keystroke
            .key
            .chars()
            .next()
            .filter(|letter| keystroke.key.chars().count() == 1 && letter.is_ascii_lowercase());
        let Some(letter) = letter else {
            self.exit_jump(cx);
            return;
        };
        let Some(jump) = &mut self.jump else {
            return;
        };
        match jump.press(letter) {
            Pressed::Jump(id) => {
                self.exit_jump(cx);
                self.go_to(&id, 1.0, window, cx);
            }
            Pressed::Typed | Pressed::Ignored => cx.notify(),
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "space" && self.space_held {
            self.space_held = false;
            cx.notify();
        }
    }

    // ---- pointer input (called from the element's window-level listeners) ---------------

    fn gesture_config(&self) -> GestureConfig {
        GestureConfig {
            camera_locked: self.camera_locked,
            space_held: self.space_held,
        }
    }

    fn reduce(&mut self, input: gesture::Input, window: &mut Window, cx: &mut Context<Self>) {
        // The scrim swallows the pointer while labels are up; the canvas' window-level
        // listeners would otherwise still pan and marquee behind it.
        if self.jump.is_some() {
            return;
        }
        let selected = self.document.read(cx).selected().clone();
        let config = self.gesture_config();
        let effects = gesture::reduce(
            &mut self.interaction,
            &config,
            self.camera,
            &selected,
            input,
        );
        if effects.is_empty() {
            // A stroke emits nothing until the pen lifts, but the live preview has to follow the
            // pen, so the sample itself is the state change that earns the repaint.
            if !self.interaction.stroke_world().is_empty() {
                cx.notify();
            }
            return;
        }
        for effect in effects {
            self.apply(effect, window, cx);
        }
        cx.notify();
    }

    fn apply(&mut self, effect: Effect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            Effect::Pan(delta) => {
                self.flight = None;
                self.camera = self.camera.panned_by(delta);
                self.nudge_peekers(cx);
            }
            Effect::ZoomAbout { anchor, factor } => {
                self.flight = None;
                self.camera = self.camera.zoomed_by_about(anchor, factor);
                self.nudge_peekers(cx);
            }
            Effect::MarqueeChanged { world, baseline } => {
                self.document.update(cx, |document, cx| {
                    let hits = peek_canvas::hit::nodes_in_rect(document.nodes(), world)
                        .cloned()
                        .collect::<BTreeSet<NodeId>>();
                    if document.select_only(baseline.into_iter().chain(hits)) {
                        cx.notify();
                    }
                });
            }
            Effect::TranslateNodes { ids, delta } => {
                self.document.update(cx, |document, cx| {
                    document.translate_nodes(&ids, delta);
                    cx.notify();
                });
            }
            Effect::SelectOnly(ids) => {
                self.update_selection(cx, |document| document.select_only(ids));
            }
            Effect::ExtendSelection(ids) => {
                self.update_selection(cx, |document| document.extend_selection(ids));
            }
            Effect::ToggleSelect(id) => {
                self.update_selection(cx, |document| document.toggle_selected(&id));
            }
            Effect::SelectEdgeOnly(id) => {
                self.update_selection(cx, |document| document.select_edge_only(id));
            }
            Effect::ToggleEdgeSelect(id) => {
                self.update_selection(cx, |document| document.toggle_edge_selected(&id));
            }
            Effect::DeselectAll => self.update_selection(cx, Document::deselect_all),
            Effect::ResizeNode { id, bounds } => {
                self.document.update(cx, |document, cx| {
                    document.set_bounds(&id, bounds);
                    cx.notify();
                });
            }
            Effect::PlaceNode { node_type, world } => self.place_node(node_type, world, cx),
            Effect::ResizePlacement { world } => {
                let Some(id) = self.placement.clone() else {
                    return;
                };
                self.document.update(cx, |document, cx| {
                    document.resize_placement(&id, world);
                    cx.notify();
                });
            }
            Effect::CommitPlacement => self.commit_placement(window, cx),
            // Not `place_node`: `useDrawTool` neither selects the stroke nor flies the camera to
            // it, and the tool is left armed for the next one.
            Effect::PlaceDrawing { points } => {
                self.document.update(cx, |document, cx| {
                    if document.create_drawing(&points).is_some() {
                        document.checkpoint();
                        cx.notify();
                    }
                });
            }
            Effect::GestureEnded => {
                // Seal the undo transaction the drag or resize opened, so the next gesture is
                // its own entry however quickly it follows.
                self.document
                    .update(cx, |document, _| document.checkpoint());
                self.commit_viewport(cx);
            }
        }
    }

    fn update_selection(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Document) -> bool,
    ) {
        self.document.update(cx, |document, cx| {
            if change(document) {
                cx.notify();
            }
        });
    }

    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let button = match event.button {
            MouseButton::Left => gesture::Button::Left,
            MouseButton::Middle => gesture::Button::Middle,
            MouseButton::Right => gesture::Button::Right,
            MouseButton::Navigate(_) => return,
        };
        let screen = self.to_pane(event.position);
        let on = self.hit_at(screen, cx);
        self.reduce(
            gesture::Input::Down {
                button,
                screen,
                modifiers: modifiers(event.modifiers),
                on,
            },
            window,
            cx,
        );
    }

    pub(crate) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.is_idle() {
            self.hover(event.position, event.modifiers.secondary(), cx);
            return;
        }
        let screen = self.to_pane(event.position);
        self.reduce(gesture::Input::Move { screen }, window, cx);
    }

    pub(crate) fn mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let button = match event.button {
            MouseButton::Left => gesture::Button::Left,
            MouseButton::Middle => gesture::Button::Middle,
            MouseButton::Right => gesture::Button::Right,
            MouseButton::Navigate(_) => return,
        };
        let screen = self.to_pane(event.position);
        self.reduce(gesture::Input::Up { button, screen }, window, cx);
    }

    pub(crate) fn scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event
            .delta
            .pixel_delta(convert::pixels(gesture::LINE_HEIGHT));
        let precise = matches!(event.delta, ScrollDelta::Pixels(_));
        self.reduce(
            gesture::Input::Wheel {
                screen: self.to_pane(event.position),
                delta: convert::from_pixel_point(delta),
                modifiers: modifiers(event.modifiers),
                phase: phase(event.touch_phase),
            },
            window,
            cx,
        );
        if !precise || event.touch_phase == TouchPhase::Moved {
            self.schedule_wheel_commit(cx);
        }
    }

    pub(crate) fn pinch(
        &mut self,
        event: &PinchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reduce(
            gesture::Input::Pinch {
                screen: self.to_pane(event.position),
                delta: f64::from(event.delta),
                phase: phase(event.phase),
            },
            window,
            cx,
        );
    }

    /// What the idle pointer is over. The move listener is window-level, so it fires while
    /// the pointer is over the dock or a dialog too; those positions clear the hover rather
    /// than hit-testing through the chrome.
    fn hover(
        &mut self,
        position: gpui_kit::Point<Pixels>,
        drag_anywhere: bool,
        cx: &mut Context<Self>,
    ) {
        let inside = self
            .pane_bounds
            .is_some_and(|bounds| bounds.contains(&position));
        let hovered = Hover {
            region: inside
                .then(|| self.node_hit_at(self.to_pane(position), cx))
                .flatten()
                .map(|hit| hit.region),
            drag_anywhere,
        };
        if hovered == self.hovered {
            return;
        }
        self.hovered = hovered;
        cx.notify();
    }

    /// Which node and region the pointer is over, in world space.
    ///
    /// Resolved here rather than by listeners on the node element: `CanvasElement` registers
    /// its window-level listeners last and gpui dispatches the bubble phase in reverse
    /// registration order, so the canvas always hears a press first.
    fn node_hit_at(&self, screen: Point, cx: &App) -> Option<peek_canvas::hit::NodeHit> {
        let world = self.camera.screen_to_world(screen);
        peek_canvas::hit::node_hit_at(self.document.read(cx).nodes(), world, self.camera.zoom)
    }

    /// The same question a press asks, widened to the edge layer beneath the nodes.
    fn hit_at(&self, screen: Point, cx: &App) -> Option<peek_canvas::hit::Hit> {
        let world = self.camera.screen_to_world(screen);
        peek_canvas::hit::hit_at(
            self.document.read(cx).active_page(),
            world,
            self.camera.zoom,
        )
    }

    /// The stroke under the pen, converted out of the world samples every frame so a pan or a
    /// zoom mid-stroke moves the ink with the canvas rather than with the cursor.
    ///
    /// The colour and width are `peek_canvas`'s own commit constants, so the preview cannot
    /// drift from the node it turns into.
    fn live_stroke(&self, cx: &App) -> Option<element::LiveStroke> {
        let samples = self.interaction.stroke_world();
        if samples.len() < 2 {
            return None;
        }
        let points = samples
            .iter()
            .map(|sample| {
                let screen = self.camera.world_to_screen(*sample);
                // Pressure is stored but never read: the renderer simulates it from sample
                // spacing, as `DrawNode.tsx` does by leaving `simulatePressure` at its default.
                [screen.x, screen.y, 0.5]
            })
            .collect();
        Some(element::LiveStroke {
            points,
            // The committed node scales with the camera; this is painted outside that layer, so
            // `LiveStroke.tsx` multiplies the zoom in by hand and so do we.
            size: peek_canvas::DRAW_STROKE_WIDTH
                * node::draw::SIZE_PER_STROKE_WIDTH
                * self.camera.zoom,
            color: node::draw::color::resolve(peek_canvas::DRAW_COLOR, cx.peek_theme()),
        })
    }

    /// Creates the node a placement is sizing. The undo step it opens is left open until
    /// [`CanvasView::commit_placement`] seals it, so the whole drag is one entry.
    fn place_node(&mut self, node_type: NodeType, world: Rect, cx: &mut Context<Self>) {
        let id = self.document.update(cx, |document, cx| {
            let id = document.create_node(node_type, world);
            document.select_only([id.clone()]);
            cx.notify();
            id
        });
        log::debug!("peek: placing {node_type:?} as {id}");
        self.placement = Some(id);
    }

    /// Ends a placement: one undo step, the node framed at 100 % as `usePlaceTool` does, and a
    /// query handed straight to its editor so it is typable without a further click.
    fn commit_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.placement.take() else {
            return;
        };
        let Some(node) = self.document.update(cx, |document, _| {
            document.checkpoint();
            document.node(&id).cloned()
        }) else {
            return;
        };
        let (pane, top) = self.framing_pane(window);
        let target =
            Self::below_chrome(Camera::centered_on(node.bounds().center(), 1.0, pane), top);
        self.fly_to(target, durations::ZOOM_TO_NODE, window, cx);
        if matches!(node.kind, NodeKind::Query(_)) {
            self.focus_query_editor(&node, window, cx);
        }
    }

    /// Drops the node of a placement the user escaped out of, leaving nothing to undo.
    fn cancel_placement(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.placement.take() else {
            return;
        };
        self.document.update(cx, |document, cx| {
            document.cancel_placement(&id);
            cx.notify();
        });
    }

    /// Builds one node's element tree: the shared shell around a per-kind body, or a bare
    /// body for the kinds that draw their own card.
    /// Builds an element for every node the camera can see, and collects the rects the
    /// selection ring is painted around.
    ///
    /// Split out of `render` because it is where the frame's cost actually is: everything else
    /// there is chrome that does not scale with the page.
    fn node_items(
        &mut self,
        visible: Rect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<NodeItem>, Vec<(Rect, Hsla)>) {
        // Cloned so the per-kind bodies below can take `&mut App`: `Entity::read` borrows it.
        // Only what the camera can see is cloned: an agent node carries its whole message
        // history, tool arguments and all, and a page of them is megabytes a frame otherwise.
        let document = self.document.read(cx);
        let total_nodes = document.nodes().len();
        let nodes: Vec<peek_document::Node> = document
            .nodes()
            .iter()
            .filter(|node| node.bounds().intersects(visible))
            .cloned()
            .collect();
        let selection = document.selected().clone();
        // Node membership only moves when the document does, and every route that adds or
        // removes one bumps the revision — page switch and undo included. Collecting the ids
        // per frame would be a string hash per node for an answer that almost never changes.
        let revision = document.revision();
        let live = self.node_states.needs_pruning(revision).then(|| {
            document
                .nodes()
                .iter()
                .map(|node| node.id.clone())
                .collect()
        });
        if let Some(live) = live {
            self.node_states.retain_live(&live, revision, cx);
        }
        self.detail = self.resolved_detail(window, cx);
        self.reclaim_focus(window, cx);

        let ring = cx.peek_theme().clone();
        let dim = wayfinding::dim(self.camera.zoom).nodes;
        let mut items = Vec::new();
        let mut selected_rects = Vec::new();
        for node in &nodes {
            let world = node.bounds();
            let selected = selection.contains(&node.id);
            if selected {
                selected_rects.push((world, ring.selection_ring(node.node_type())));
            }
            let element = self.node_element(node, selected, window, cx);
            items.push(NodeItem {
                world,
                // Wrapped rather than styled in place: the two kinds of node root (a bare
                // card, a `NodeShell`) have no shared builder to hang an opacity on, and at
                // working zoom this branch never runs.
                element: if dim < 1.0 {
                    div()
                        .size_full()
                        .opacity(dim)
                        .child(element)
                        .into_any_element()
                } else {
                    element
                },
            });
        }
        self.frame_stats.tick(items.len(), total_nodes);
        (items, selected_rects)
    }

    fn node_element(
        &mut self,
        node: &peek_document::Node,
        selected: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui_kit::AnyElement {
        // A selected node keeps its body whatever the camera is doing: it is the one the user
        // is working with, and it is what a `Zoom::FitSelection` is about to fly to.
        let detail = if selected { Detail::Full } else { self.detail };
        let document = self.node_states.document().clone();
        // Retained state is created on first render, so building a reduced node must not ask
        // for it: zooming out over a page of query nodes would otherwise open a language-server
        // document for every one of them. Whatever already exists is left alone — `retain_live`
        // prunes against the document, never against what is on screen.
        let state = if detail.is_reduced() && !node::kind::is_bare(node) {
            self.node_states.peek(&node.id)
        } else {
            self.node_states.get(node, window, cx)
        };
        let context = node::kind::NodeContext {
            document: &document,
            state,
            selected,
            detail,
            size: node.size(),
            zoom: render_scale(self.camera.zoom),
        };
        let body = node::kind::body(node, context, window, cx);
        if node::kind::is_bare(node) {
            // Bare kinds draw their own card, but every node still needs a stable identity for
            // hit-testing and tests.
            return div()
                .id(SharedString::from(node.id.to_string()))
                .test_support()
                .size_full()
                .child(body)
                .into_any_element();
        }
        let extras = node::kind::header_extras(node, context, window, cx);
        let closing = (document.clone(), node.id.clone());
        NodeShell::new(node, selected, body, cx)
            .header_extras(extras)
            .on_close(move |window, cx| {
                // Selecting first and then dispatching keeps the button on the same path as the
                // Delete key and the palette, rather than reaching into the document itself.
                let (document, node) = &closing;
                document.update(cx, |document, cx| {
                    document.select_only([node.clone()]);
                    cx.notify();
                });
                window.dispatch_action(
                    Box::new(crate::commands::actions::edit::DeleteSelection),
                    cx,
                );
            })
            .into_any_element()
    }

    pub(crate) fn cursor(&self) -> CursorStyle {
        match &self.interaction {
            Interaction::Panning { .. } | Interaction::DraggingNodes { .. } => {
                CursorStyle::ClosedHand
            }
            Interaction::Placing { .. } | Interaction::Drawing { .. } => CursorStyle::Crosshair,
            Interaction::ResizingNode { corner, .. } => resize_cursor(*corner),
            _ if self.space_held => CursorStyle::OpenHand,
            _ => hover_cursor(self.hovered),
        }
    }
}

/// What an idle pointer is over: the region under it, and whether the secondary modifier was
/// down at that move — cmd turns a node's whole card into a drag handle, which the cursor has
/// to advertise before the press.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Hover {
    region: Option<NodeRegion>,
    drag_anywhere: bool,
}

/// The affordance of the region under an idle pointer: `node.css` gives the header `grab` and
/// leaves the body to whatever the kind's own content wants.
fn hover_cursor(hovered: Hover) -> CursorStyle {
    match hovered.region {
        Some(NodeRegion::Resize(corner)) => resize_cursor(corner),
        Some(NodeRegion::Header) => CursorStyle::OpenHand,
        // With the secondary modifier down the body is a drag handle too.
        Some(NodeRegion::Body) if hovered.drag_anywhere => CursorStyle::OpenHand,
        Some(NodeRegion::Body) | None => CursorStyle::Arrow,
    }
}

fn resize_cursor(corner: Corner) -> CursorStyle {
    match corner {
        Corner::Top | Corner::Bottom => CursorStyle::ResizeUpDown,
        Corner::Left | Corner::Right => CursorStyle::ResizeLeftRight,
        Corner::TopLeft | Corner::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        Corner::TopRight | Corner::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

fn modifiers(modifiers: Modifiers) -> gesture::Modifiers {
    gesture::Modifiers {
        shift: modifiers.shift,
        secondary: modifiers.secondary(),
        control: modifiers.control,
    }
}

fn phase(phase: TouchPhase) -> gesture::Phase {
    match phase {
        TouchPhase::Started => gesture::Phase::Started,
        TouchPhase::Moved => gesture::Phase::Moved,
        TouchPhase::Ended | TouchPhase::Cancelled => gesture::Phase::Ended,
    }
}

impl CanvasView {
    /// One [`EdgeItem`] per visible edge, tinted by the kind it points at.
    fn edge_items(&self, visible: Rect, cx: &App) -> Vec<EdgeItem> {
        let theme = cx.peek_theme();
        let document = self.document.read(cx);
        let page = document.active_page();
        // `Page::node` is a linear scan, and every edge needs two of them: on a page with as
        // many edges as nodes that is quadratic work for a lookup, every frame.
        let by_id: HashMap<&NodeId, &peek_document::Node> =
            page.nodes.iter().map(|node| (&node.id, node)).collect();
        page.edges
            .iter()
            .filter_map(|edge| {
                let source = *by_id.get(&edge.source)?;
                let target = *by_id.get(&edge.target)?;
                let curve = curve_between(source.bounds(), target.bounds());
                if !curve.bounds().intersects(visible) {
                    return None;
                }
                let state = edge_state(document, edge);
                Some(EdgeItem {
                    curve,
                    color: theme.edge(target.node_type(), state),
                    width: edges::stroke_width(state),
                })
            })
            .collect()
    }
}

/// `node.css` layers three rules over an edge; the later `.connection-active` one wins where
/// both apply, which is why a selected edge touching a selected node reads as connected.
fn edge_state(document: &Document, edge: &Edge) -> EdgeState {
    if document.is_selected(&edge.source) || document.is_selected(&edge.target) {
        return EdgeState::ConnectionActive;
    }
    if document.is_edge_selected(&edge.id) {
        return EdgeState::Selected;
    }
    EdgeState::Resting
}

impl Render for CanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick_flight(window, cx);
        self.tick_layout(window, cx);
        self.tick_flash(window, cx);
        self.arm_fps_settle(cx);

        let started = self.frame_stats.enabled().then(Instant::now);
        let camera = self.camera;
        let pane = self.pane_size(window);
        let visible = camera
            .visible_world_rect(pane)
            .dilated(CULL_MARGIN_PX / camera.zoom);
        let theme = cx.peek_theme().clone();

        let (items, selected_rects) = self.node_items(visible, window, cx);
        let regions = self.derived_regions(cx);
        let nodes = self.document.read(cx).nodes().to_vec();

        let overlay = Overlay {
            edges: self.edge_items(visible, cx),
            regions: wayfinding::halos(self, &regions, &nodes, cx),
            edge_dim: wayfinding::dim(camera.zoom).edges,
            selected_rects,
            marquee: self.interaction.marquee_screen_rect(),
            stroke: self.live_stroke(cx),
            background: theme.canvas_base,
            gradient: theme.canvas_gradient,
            grid_dot: theme.bg_grid,
            marquee_fill: theme.accent_bg,
            marquee_border: theme.selection,
            node_radius: f32::from(theme.radius_node),
        };
        let canvas = CanvasElement::new(cx.entity(), camera, window.rem_size(), items, overlay)
            .cursor(self.cursor());

        let element = div()
            .id("canvas")
            .test_support()
            .relative()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(if self.jump.is_some() {
                commands::CANVAS_JUMPING
            } else {
                commands::CANVAS
            })
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::fit_view))
            .on_action(cx.listener(Self::fit_selection))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::previous_page))
            .on_action(cx.listener(Self::next_page))
            .on_action(cx.listener(Self::toggle_camera_lock))
            .on_action(cx.listener(Self::delete_selection))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::new_page))
            .on_action(cx.listener(Self::close_page))
            .on_action(cx.listener(Self::place_query))
            .on_action(cx.listener(Self::place_agent))
            .on_action(cx.listener(Self::fork_agent))
            .on_action(cx.listener(Self::place_text))
            .on_action(cx.listener(Self::place_variable))
            .on_action(cx.listener(Self::place_drawing))
            .on_action(cx.listener(Self::go_to_node))
            .on_action(cx.listener(Self::select_node_left))
            .on_action(cx.listener(Self::select_node_right))
            .on_action(cx.listener(Self::select_node_up))
            .on_action(cx.listener(Self::select_node_down))
            .on_action(cx.listener(Self::focus_query));
        let element = dispatch::register(element, cx);
        let element = page_search::register(element, cx);
        let element = element
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .child(div().absolute().inset_0().child(canvas))
            .when(self.chrome_visible, |this| {
                this.child(hud::render(self, cx))
                    .child(toolbar::render(self, window, cx))
            })
            // Last, so the scrim dims the HUD and toolbar too.
            .children(
                self.jump
                    .as_ref()
                    .map(|jump| jump::render(jump, camera, cx)),
            )
            .children(page_search::render(self, cx))
            // Under the transient surfaces below but over the HUD, as `WayfindingLayer` sits
            // over the React Flow panels: a beacon is what you navigate by when the cards have
            // receded, so nothing but a menu should cover it.
            .children(wayfinding::render(
                self,
                &regions,
                wayfinding::Frame { camera, pane },
                cx,
            ))
            .child(self.suggestions.clone())
            .child(self.regions.clone())
            // Last of all: the menu is the most transient surface on the canvas, and a press
            // anywhere outside it has to reach its scrim before anything else.
            .children(context_menu::render(self, cx))
            // Above the menu: a cell's menu can raise this, and the menu closes as it does.
            .children(json_editor::render(self, cx));

        if let Some(started) = started {
            self.frame_stats.record(Phase::Render, started.elapsed());
        }
        element
    }
}

pub(crate) fn screen_rect(camera: Camera, world: Rect) -> Bounds<Pixels> {
    to_pixel_bounds(camera.world_rect_to_screen(world))
}

pub(crate) fn screen_point(camera: Camera, world: Point) -> gpui_kit::Point<Pixels> {
    to_pixel_point(camera.world_to_screen(world))
}
