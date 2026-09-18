//! The custom element that paints the canvas and places node shells. Nodes are real element
//! trees laid out at `AvailableSpace::Definite(size * zoom)` inside a rem scope of
//! `base_rem * zoom`, so text and padding scale with the camera while hitboxes stay in
//! screen space for free. Pointer input is registered window-wide and gated on the view's
//! interaction state, so pans and marquees keep receiving events across node hitboxes.

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, AvailableSpace, BorderStyle, Bounds, ContentMask, CursorStyle, DispatchPhase,
    ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, LayoutId,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point, ScrollWheelEvent,
    Style, Window, fill, linear_color_stop, linear_gradient, px, quad, relative, size,
    transparent_black,
};
use peek_canvas::render_scale;
use peek_canvas::stroke::{self as peek_stroke, StrokeOptions};
use peek_canvas::{Camera, Rect};

use crate::node::draw::{Placement, tessellate};

use super::convert::to_pixel_bounds;
use super::edges::{self, EdgeItem};
use super::frame_stats::Phase;
use super::{CanvasView, grid, screen_rect};

/// Selection ring: constant screen width and offset regardless of zoom (`node.css`).
const RING_WIDTH: f32 = 1.5;
const RING_OFFSET: f32 = 3.0;

/// The stroke being drawn, as `LiveStroke.tsx` has it: pane-relative screen points, and a
/// diameter already multiplied by the zoom, because this is painted outside the node layer and
/// so gets none of the camera's scaling for free.
pub(crate) struct LiveStroke {
    pub points: Vec<[f64; 3]>,
    pub size: f64,
    pub color: Hsla,
}

pub(crate) struct NodeItem {
    pub world: Rect,
    pub element: AnyElement,
}

pub(crate) struct Overlay {
    /// Painted behind every node, so a curve runs under the cards it connects.
    pub edges: Vec<EdgeItem>,
    /// Each selected node's rect with the colour its outline is drawn in, which `node.css`
    /// takes from the node's own kind (`--pk-node-type-color`).
    pub selected_rects: Vec<(Rect, Hsla)>,
    pub marquee: Option<Rect>,
    pub stroke: Option<LiveStroke>,
    pub background: Hsla,
    pub gradient: Option<(Hsla, Hsla)>,
    pub grid_dot: Hsla,
    pub marquee_fill: Hsla,
    pub marquee_border: Hsla,
    /// Node corner radius in world pixels (zoom 1).
    pub node_radius: f32,
}

pub(crate) struct CanvasElement {
    view: Entity<CanvasView>,
    camera: Camera,
    base_rem: Pixels,
    items: Vec<NodeItem>,
    overlay: Overlay,
    cursor: CursorStyle,
}

impl CanvasElement {
    pub(crate) fn new(
        view: Entity<CanvasView>,
        camera: Camera,
        base_rem: Pixels,
        items: Vec<NodeItem>,
        overlay: Overlay,
    ) -> Self {
        Self {
            view,
            camera,
            base_rem,
            items,
            overlay,
            cursor: CursorStyle::Arrow,
        }
    }

    pub(crate) fn cursor(mut self, cursor: CursorStyle) -> Self {
        self.cursor = cursor;
        self
    }

    /// Whether the view is collecting frame timings, asked before an `Instant::now`.
    fn timing(&self, cx: &App) -> bool {
        self.view.read(cx).frame_stats_enabled()
    }

    fn record(&self, phase: Phase, started: Option<std::time::Instant>, cx: &mut App) {
        let Some(started) = started else {
            return;
        };
        let elapsed = started.elapsed();
        self.view
            .update(cx, |view, _| view.record_frame_phase(phase, elapsed));
    }

    /// The rem size node contents are laid out at.
    ///
    /// Snapped, not the raw zoom: gpui keys its line-layout and glyph caches on the exact font
    /// size, so a continuously changing rem re-shapes every visible string every frame. The
    /// node's *box* still uses the exact zoom — see [`peek_canvas::render_scale`].
    fn node_rem(&self) -> Pixels {
        #[allow(clippy::cast_possible_truncation, reason = "scale is within 0.1..=4")]
        let scale = render_scale(self.camera.zoom) as f32;
        self.base_rem * scale
    }
}

pub(crate) struct CanvasPrepaint {
    hitbox: Hitbox,
}

impl IntoElement for CanvasElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CanvasElement {
    type RequestLayoutState = ();
    type PrepaintState = CanvasPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let started = self.timing(cx).then(std::time::Instant::now);
        self.view
            .update(cx, |view, cx| view.set_pane_bounds(bounds, cx));
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let rem = self.node_rem();
        let camera = self.camera;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for NodeItem { world, element } in &mut self.items {
                let screen = screen_rect(camera, *world);
                let origin = bounds.origin + screen.origin;
                let available = size(
                    AvailableSpace::Definite(screen.size.width),
                    AvailableSpace::Definite(screen.size.height),
                );
                window.with_rem_size(Some(rem), |window| {
                    element.layout_as_root(available, window, cx);
                    element.prepaint_at(origin, window, cx);
                });
            }
        });

        self.record(Phase::Prepaint, started, cx);
        CanvasPrepaint { hitbox }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let started = self.timing(cx).then(std::time::Instant::now);
        let camera = self.camera;
        let rem = self.node_rem();
        let radius = self.node_radius(camera);

        // Registered *before* the node elements paint, so their own wheel listeners land later
        // in the frame's list. gpui bubbles in reverse registration order, which hands the
        // event to a hovered node first; gpui-base's editor stops propagation only when it
        // actually scrolled, so an already-scrollable body absorbs the wheel and everything
        // else falls through to the canvas and pans. That is `useScrollFallthrough`'s rule.
        register_scroll_listeners(&self.view, &prepaint.hitbox, window);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, self.overlay.background));
            if let Some((top, bottom)) = self.overlay.gradient {
                let gradient = linear_gradient(
                    180.0,
                    linear_color_stop(top, 0.0),
                    linear_color_stop(bottom, 1.0),
                );
                window.paint_quad(fill(bounds, gradient));
            }
            grid::paint_dot_grid(bounds, camera, self.overlay.grid_dot, window);
            edges::paint(bounds, camera, &self.overlay.edges, window);

            for NodeItem { element, .. } in &mut self.items {
                window.with_rem_size(Some(rem), |window| element.paint(window, cx));
            }

            for (world, color) in &self.overlay.selected_rects {
                let screen = offset(screen_rect(camera, *world), bounds.origin);
                let ring = screen.dilate(px(RING_OFFSET));
                window.paint_quad(quad(
                    ring,
                    px(radius + RING_OFFSET),
                    transparent_black(),
                    px(RING_WIDTH),
                    *color,
                    BorderStyle::Solid,
                ));
            }

            // Above everything, which is `LiveStroke.tsx`'s `zIndex: 1000`: the ink has to
            // read over the nodes it is being drawn across.
            if let Some(stroke) = &self.overlay.stroke {
                paint_live_stroke(stroke, bounds.origin, window);
            }

            if let Some(marquee) = self.overlay.marquee {
                let screen = offset(to_pixel_bounds(marquee), bounds.origin);
                window.paint_quad(quad(
                    screen,
                    px(4.0),
                    self.overlay.marquee_fill,
                    px(1.0),
                    self.overlay.marquee_border,
                    BorderStyle::Solid,
                ));
            }
        });

        window.set_cursor_style(self.cursor, &prepaint.hitbox);
        register_pointer_listeners(&self.view, &prepaint.hitbox, window);
        self.record(Phase::Paint, started, cx);
    }
}

/// Wheel and pinch. Registered first so node bodies get the chance to absorb a scroll; see the
/// comment at the call site. `cmd`/`ctrl` + wheel is taken in the capture phase, where the
/// canvas *is* first, so a zoom is never swallowed by an editor that happens to be hovered —
/// the reference bails out of absorption on `ctrlKey` for the same reason.
fn register_scroll_listeners(view: &Entity<CanvasView>, hitbox: &Hitbox, window: &mut Window) {
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        move |event: &ScrollWheelEvent, phase, window, cx| {
            if !hitbox.should_handle_scroll(window) {
                return;
            }
            let zooming = event.modifiers.secondary() || event.modifiers.control;
            if phase == DispatchPhase::Capture && zooming {
                view.update(cx, |view, cx| view.scroll_wheel(event, window, cx));
                cx.stop_propagation();
            } else if phase == DispatchPhase::Bubble && !zooming {
                view.update(cx, |view, cx| view.scroll_wheel(event, window, cx));
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        move |event: &PinchEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                view.update(cx, |view, cx| view.pinch(event, window, cx));
            }
        }
    });
}

/// Press, move and release. Registered last, so the canvas hears a press before any node and
/// the world-space hit test in `peek_canvas::hit` stays the single source of truth.
fn register_pointer_listeners(view: &Entity<CanvasView>, hitbox: &Hitbox, window: &mut Window) {
    window.on_mouse_event({
        let view = view.clone();
        let hitbox = hitbox.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                view.update(cx, |view, cx| view.mouse_down(event, window, cx));
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |view, cx| view.mouse_move(event, window, cx));
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |view, cx| view.mouse_up(event, window, cx));
            }
        }
    });
}

/// The same tessellation the committed node uses, so the stroke does not change shape at the
/// moment the pen lifts. The points are already in screen units, hence `scale: 1.0`.
fn paint_live_stroke(stroke: &LiveStroke, origin: Point<Pixels>, window: &mut Window) {
    let options = StrokeOptions {
        size: stroke.size,
        ..StrokeOptions::default()
    };
    let Some(outline) = peek_stroke::path(&peek_stroke::outline(&stroke.points, options)) else {
        return;
    };
    let Some(path) = tessellate(&outline, Placement { origin, scale: 1.0 }) else {
        return;
    };
    window.paint_path(path, stroke.color);
}

fn offset(bounds: Bounds<Pixels>, by: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(bounds.origin + by, bounds.size)
}

impl CanvasElement {
    fn node_radius(&self, camera: Camera) -> f32 {
        #[allow(clippy::cast_possible_truncation, reason = "radius is a few pixels")]
        let radius = (f64::from(self.overlay.node_radius) * camera.zoom) as f32;
        radius
    }
}
