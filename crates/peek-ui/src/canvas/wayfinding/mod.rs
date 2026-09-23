//! Regions on screen: the beacons a zoomed-out canvas is navigated by, the peekers that point
//! at the ones off screen, and the card that asks what to do with a suggestion.
//!
//! The port of `~/labs/peek/src/canvas/wayfinding/`. Everything here is **chrome**: it is
//! rendered as a late child of [`CanvasView`], outside the camera's rem scope, so a label keeps
//! a constant screen size at every zoom — a way of reaching a region is not part of it. The one
//! exception is the halo, which *is* the region's box and so is painted in world space by
//! [`halos`].
//!
//! The layer is gated three ways, mirroring `WayfindingLayer.tsx`'s two-stage split: regions
//! must be enabled in `settings.json`, the chrome must be visible, and the page must actually
//! have a region with a live member. A page without regions builds nothing at all.

mod beacons;
pub(crate) mod card;
pub(crate) mod halos;
pub(crate) mod menu;
mod peekers;

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{App, Context, MouseButton, MouseUpEvent, Task, Window, div};
use peek_canvas::camera::FitOptions;
use peek_canvas::flight::durations;
use peek_canvas::regions::crossfade::{self, cross_fade};
use peek_canvas::regions::derive::{self, Derived};
use peek_canvas::{Camera, Point, Size};
use peek_document::{NodeId, RegionId, RegionStatus};
use peek_theme::ActivePeekTheme;

use super::CanvasView;
use crate::settings::Settings;

/// How long the ring marking a fold stays up — `state.ts`'s `FLASH_MS`.
const FLASH: Duration = Duration::from_millis(900);
/// How long after the last camera change peekers keep showing — `useViewportMotion`'s `IDLE_MS`.
const MOTION_IDLE: Duration = Duration::from_millis(900);
/// Pointer travel before a beacon press is a drag rather than a click to enter.
const DRAG_THRESHOLD: f64 = 3.0;
/// How much room a flight leaves around a region's members — `useRegionActions.ts`'s
/// `zoomToNodes(ids, { padding: 0.15 })`.
const FIT_PADDING: f64 = 0.15;

/// A beacon drag in flight. Regions store no position of their own, so this moves the members
/// and lets the box re-derive under them.
struct BeaconDrag {
    /// Which region was pressed, so a press that never moved can enter it on release.
    region: RegionId,
    members: Vec<NodeId>,
    last: Point,
}

/// Session state for the wayfinding layer. None of it is persisted: it is all about where the
/// pointer and the camera are right now.
#[derive(Default)]
pub(crate) struct Wayfinding {
    /// The region a fold just landed in, and when its ring is done.
    flash: Option<(RegionId, Instant)>,
    /// When the peekers were last woken: by a camera change, or by the pointer resting on one
    /// of them. They are a transient compass, so they fade [`MOTION_IDLE`] after the last nudge
    /// — and holding them up on hover is what stops reaching for a label being a race the
    /// pointer loses.
    nudged_at: Option<Instant>,
    drag: Option<BeaconDrag>,
    /// Whether the last press on a beacon turned into a drag. The `click` that follows a drag
    /// arrives after the release has already cleared [`Self::drag`], so this is what stops a
    /// dragged beacon from also flying the camera to where it just landed.
    dragged: bool,
    /// One deferred repaint so the peekers can fade out after the camera settles, re-armed on
    /// every move — the shape `fps_settle` already uses.
    settle: Option<Task<()>>,
}

impl std::fmt::Debug for Wayfinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Wayfinding")
            .field("flash", &self.flash.as_ref().map(|(id, _)| id))
            .field("dragging", &self.drag.is_some())
            .finish_non_exhaustive()
    }
}

impl CanvasView {
    /// The active page's regions with their live members, or empty when the feature is off.
    ///
    /// Recomputed per frame rather than cached: a region's box is its members' bounds, so any
    /// cache would have to be invalidated by every drag, resize and undo.
    pub(crate) fn derived_regions(&self, cx: &App) -> Vec<Derived> {
        if !Settings::get(cx).canvas.enable_regions {
            return Vec::new();
        }
        let document = self.shown().read(cx);
        derive::derive(document.nodes(), document.regions())
    }

    /// Marks the region a selection just folded into. Folding keeps the target's name, so
    /// nothing else says what absorbed the nodes.
    pub(crate) fn flash_region(&mut self, region: RegionId, cx: &mut Context<Self>) {
        self.wayfinding.flash = Some((region, Instant::now() + FLASH));
        cx.notify();
    }

    /// Frames the region's live members, at the reference's padding and duration.
    pub(crate) fn fly_to_region(
        &mut self,
        region: &RegionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let members = self
            .document
            .read(cx)
            .regions()
            .iter()
            .find(|candidate| &candidate.id == region)
            .map(|region| region.member_ids.clone())
            .unwrap_or_default();
        self.frame_nodes(&members, window, cx);
    }

    /// Flies so every one of `members` that still exists is in frame. A region whose members
    /// are all gone is not an error — it simply has nowhere to fly to.
    fn frame_nodes(&mut self, members: &[NodeId], window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.document.read(cx).bounds_of(members.iter()) else {
            return;
        };
        let (pane, top) = self.framing_pane(window);
        let target = Self::below_chrome(
            Camera::fit_bounds(bounds, pane, FitOptions::padding(FIT_PADDING)),
            top,
        );
        self.fly_to(target, durations::REGION_FLY, window, cx);
    }

    /// Wakes the peekers and re-arms their quiet period. Called wherever the camera changes —
    /// which is what `useViewportMotion` watches the transform for — and while the pointer
    /// rests on a peeker.
    pub(crate) fn nudge_peekers(&mut self, cx: &mut Context<Self>) {
        self.wayfinding.nudged_at = Some(Instant::now());
        // Re-arming drops the pending task, so a burst of camera changes still costs exactly
        // one trailing repaint — the shape `fps_settle` already uses.
        self.wayfinding.settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MOTION_IDLE).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        }));
    }

    fn peekers_showing(&self) -> bool {
        self.wayfinding
            .nudged_at
            .is_some_and(|at| at.elapsed() < MOTION_IDLE)
    }

    /// Keeps the frames coming while a flash ring is fading, and drops it once it has.
    ///
    /// Nothing else on the canvas is moving when a fold happens, so without this the ring would
    /// be painted once at full strength and then sit there until the next unrelated repaint.
    pub(super) fn tick_flash(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some((_, until)) = &self.wayfinding.flash else {
            return;
        };
        if *until <= Instant::now() {
            self.wayfinding.flash = None;
            cx.notify();
            return;
        }
        window.request_animation_frame();
    }

    /// How much of the flash ring is left for `region`, or `None` once it has finished.
    fn flash_alpha(&self, region: &RegionId) -> Option<f32> {
        let (flashing, until) = self.wayfinding.flash.as_ref()?;
        if flashing != region {
            return None;
        }
        let now = Instant::now();
        let left = until.checked_duration_since(now)?;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a ratio of two short durations"
        )]
        Some((left.as_secs_f64() / FLASH.as_secs_f64()) as f32)
    }
}

/// How much of the canvas' own content still shows while the beacons come in.
#[derive(Debug, Clone, Copy)]
pub(super) struct Dim {
    pub nodes: f32,
    pub edges: f32,
}

/// `wayfinding.css`'s `[data-wf-lowzoom]`: past the threshold the cards and curves recede and
/// the region labels carry the board. It is a step rather than a ramp there too — the CSS
/// transitions between the two states rather than interpolating with the zoom.
pub(super) fn dim(zoom: f64) -> Dim {
    if cross_fade(zoom) > crossfade::DIM_THRESHOLD_T {
        Dim {
            nodes: crossfade::NODE_DIM,
            edges: crossfade::EDGE_DIM,
        }
    } else {
        Dim {
            nodes: 1.0,
            edges: 1.0,
        }
    }
}

/// The world-space boxes, in paint order. Empty above the fade threshold for a page whose
/// regions are all confirmed and unflashed, which is the common case at working zoom.
pub(super) fn halos(
    view: &CanvasView,
    regions: &[Derived],
    nodes: &[peek_document::Node],
    cx: &App,
) -> Vec<halos::Halo> {
    let theme = cx.peek_theme();
    #[allow(clippy::cast_possible_truncation, reason = "an opacity in 0..=1")]
    let confirmed_alpha = cross_fade(view.camera.zoom) as f32;

    regions
        .iter()
        .flat_map(|region| {
            let world = region.bbox(nodes);
            let color = theme.region(region.color_index as usize);
            let mut boxes = Vec::new();
            let kind = match region.status {
                RegionStatus::Suggested => Some(halos::HaloKind::Suggested),
                RegionStatus::Confirmed if confirmed_alpha > 0.0 => {
                    Some(halos::HaloKind::Confirmed {
                        alpha: confirmed_alpha,
                    })
                }
                RegionStatus::Confirmed => None,
            };
            boxes.extend(kind.map(|kind| halos::Halo {
                world,
                color,
                background: theme.bg,
                kind,
            }));
            // After the halo, so a fold is visible over its own region's pool.
            boxes.extend(view.flash_alpha(&region.id).map(|alpha| halos::Halo {
                world,
                color,
                background: theme.bg,
                kind: halos::HaloKind::Flash { alpha },
            }));
            boxes
        })
        .collect()
}

/// The screen-space layer: beacons, peekers, and the pointer capture a beacon drag needs.
///
/// `None` when there is nothing to draw, so a page without regions costs one `is_empty`.
pub(super) fn render(
    view: &CanvasView,
    regions: &[Derived],
    frame: Frame,
    cx: &mut Context<CanvasView>,
) -> Option<impl IntoElement> {
    if regions.is_empty() || !view.chrome_visible {
        return None;
    }
    let t = cross_fade(frame.camera.zoom);
    let nodes = view.document.read(cx).nodes().to_vec();

    Some(
        div()
            .absolute()
            .inset_0()
            .children(beacons::render(view, regions, &nodes, frame, t, cx))
            .children(peekers::render(view, regions, &nodes, frame, t, cx))
            .children(drag_capture(view, cx)),
    )
}

/// What the layer needs to know about the frame it is being built for. A struct because the
/// camera and the pane always travel together here, and three of these functions would
/// otherwise be at the argument limit.
#[derive(Debug, Clone, Copy)]
pub(super) struct Frame {
    pub camera: Camera,
    pub pane: Size,
}

/// A full-window catcher, present only while a beacon is being dragged: the beacon's own
/// listeners stop firing the moment the pointer leaves it, and a drag has to keep going.
///
/// It occludes, which is also what keeps the press that ends the drag from reaching the canvas
/// and starting a marquee.
fn drag_capture(view: &CanvasView, cx: &mut Context<CanvasView>) -> Option<impl IntoElement> {
    view.wayfinding.drag.as_ref()?;
    Some(
        div()
            .id("beacon-drag-capture")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_move(cx.listener(beacons::on_drag_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view: &mut CanvasView, _: &MouseUpEvent, window, cx| {
                    beacons::end_drag(view, window, cx);
                }),
            ),
    )
}
