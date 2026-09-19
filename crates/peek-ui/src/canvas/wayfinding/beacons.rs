//! Region beacons — the port of `Beacon.tsx`, `Beacons.tsx` and `.wf-beacon`.
//!
//! A beacon is the region's title, and the only one it has: a region draws no header and no
//! label at working zoom. It appears as the camera pulls back, centred on the region's box, and
//! it is both a place to go (click) and a handle to move the region by (drag).

use gpui_kit::TestSupportExt;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString,
    Window, div, px,
};
use peek_canvas::Point;
use peek_canvas::regions::crossfade::INTERACTIVE_THRESHOLD_T;
use peek_canvas::regions::derive::Derived;
use peek_document::Node;
use peek_theme::ActivePeekTheme;

use super::{BeaconDrag, DRAG_THRESHOLD, Frame};
use crate::canvas::convert::from_pixel_point;
use crate::canvas::{CanvasView, screen_point};

/// Wide enough for a two-or-three-word region name at 26px. gpui has no self-relative
/// transform, so `translate(-50%, -50%)` becomes a fixed box centred on the point with its
/// contents centred inside it.
const WIDTH: Pixels = px(360.0);
const HEIGHT: Pixels = px(132.0);
const NAME_SIZE: Pixels = px(26.0);
const DESC_SIZE: Pixels = px(12.5);
const META_SIZE: Pixels = px(10.5);
const DOT: Pixels = px(7.0);

pub(super) fn render(
    view: &CanvasView,
    regions: &[Derived],
    nodes: &[Node],
    frame: Frame,
    t: f64,
    cx: &mut Context<CanvasView>,
) -> Option<impl IntoElement> {
    if t <= 0.0 {
        return None;
    }
    // Below the threshold a beacon is a label you read, not a control you press — and a layer
    // that took the pointer while still translucent would swallow clicks meant for the nodes
    // showing through it.
    let interactive = t > INTERACTIVE_THRESHOLD_T;
    #[allow(clippy::cast_possible_truncation, reason = "an opacity in 0..=1")]
    let opacity = t as f32;

    let beacons: Vec<_> = regions
        .iter()
        .map(|region| beacon(region, nodes, frame, interactive, view, cx))
        .collect();

    Some(
        div()
            .absolute()
            .inset_0()
            .opacity(opacity)
            .children(beacons),
    )
}

fn beacon(
    region: &Derived,
    nodes: &[Node],
    frame: Frame,
    interactive: bool,
    view: &CanvasView,
    cx: &mut Context<CanvasView>,
) -> impl IntoElement + use<> {
    let theme = cx.peek_theme().clone();
    let center = screen_point(frame.camera, region.bbox(nodes).center());
    let group = SharedString::from(format!("beacon-{}", region.id));
    let color = theme.region(region.color_index as usize);
    let dragging = view.wayfinding.drag.is_some();

    let card = div()
        .id(SharedString::from(format!("beacon-{}", region.id)))
        .test_support()
        .group(group.clone())
        .absolute()
        .left(center.x - WIDTH / 2.0)
        .top(center.y - HEIGHT / 2.0)
        .w(WIDTH)
        .h(HEIGHT)
        .v_flex()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .text_center()
        .child(
            div()
                .text_size(NAME_SIZE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg)
                .child(region.name.clone()),
        )
        .when(!region.desc.is_empty(), |this| {
            this.child(
                div()
                    .text_size(DESC_SIZE)
                    .text_color(theme.fg_muted)
                    .child(region.desc.clone()),
            )
        })
        .child(meta(region, color, &group, cx));

    if !interactive {
        return card;
    }

    let members = region.member_ids.clone();
    let id = region.id.clone();
    card.cursor(if dragging {
        gpui_kit::CursorStyle::ClosedHand
    } else {
        gpui_kit::CursorStyle::OpenHand
    })
    .hover(move |style| style.bg(theme.fg.opacity(0.05)))
    .rounded(px(14.0))
    // The canvas hears presses through window-level listeners registered before this element
    // paints, so gpui's reverse bubble order offers it here first — and stopping it is what
    // keeps a press on a beacon from also starting a marquee underneath.
    .on_mouse_down(
        MouseButton::Left,
        cx.listener(move |view, event: &MouseDownEvent, _, cx| {
            view.wayfinding.dragged = false;
            view.wayfinding.drag = Some(BeaconDrag {
                region: id.clone(),
                members: members.clone(),
                last: from_pixel_point(event.position),
            });
            cx.stop_propagation();
            cx.notify();
        }),
    )
    // No `on_mouse_up` here, and no `on_click`: the press repaints, and the frame it produces
    // carries the full-window drag catcher, which occludes this element before the release
    // arrives. Entering the region is decided in [`end_drag`] instead, off the id the press
    // recorded — which is also the only way to tell a click from the end of a drag, since the
    // release has already cleared the state by the time a click would fire.
}

/// `.bc-meta`: hidden until the beacon is hovered, so a quiet canvas stays quiet.
fn meta(
    region: &Derived,
    color: gpui_kit::Hsla,
    group: &SharedString,
    cx: &App,
) -> impl IntoElement + use<> {
    let theme = cx.peek_theme();
    div()
        .h_flex()
        .items_center()
        .gap(px(6.0))
        .mt(px(4.0))
        .opacity(0.0)
        .group_hover(group.clone(), |style| style.opacity(1.0))
        .text_size(META_SIZE)
        .font_family("Monaspace Krypton")
        .text_color(theme.fg_subtle)
        .child(div().size(DOT).rounded_full().bg(color))
        .child(format!(
            "{} nodes · drag to move · click to enter",
            region.member_ids.len()
        ))
}

/// Translates the dragged region's members. Regions store no position of their own, so moving
/// the members re-derives the box and the beacon follows — and because it goes through
/// `translate_nodes` the move is already one `EditKind::Move` undo step and already autosaves.
pub(super) fn on_drag_move(
    view: &mut CanvasView,
    event: &MouseMoveEvent,
    _: &mut Window,
    cx: &mut Context<CanvasView>,
) {
    let zoom = view.camera.zoom;
    let Some(drag) = &mut view.wayfinding.drag else {
        return;
    };
    let position = from_pixel_point(event.position);
    let travelled = position - drag.last;
    // A shaky click-to-enter should not nudge the region, so the first few pixels are free.
    if travelled.x.hypot(travelled.y) < DRAG_THRESHOLD {
        return;
    }
    drag.last = position;
    let members = drag.members.clone();
    let delta = Point::new(travelled.x / zoom, travelled.y / zoom);
    view.wayfinding.dragged = true;
    view.document.update(cx, |document, cx| {
        document.translate_nodes(&members, delta);
        cx.notify();
    });
    cx.notify();
}

/// Ends whatever the press turned out to be: a drag is sealed as its own undo entry, and a
/// press that never moved enters the region instead.
pub(super) fn end_drag(view: &mut CanvasView, window: &mut Window, cx: &mut Context<CanvasView>) {
    let Some(drag) = view.wayfinding.drag.take() else {
        return;
    };
    if view.wayfinding.dragged {
        // Seals the move, so the next drag is its own undo entry however quickly it follows.
        view.document
            .update(cx, |document, _| document.checkpoint());
        cx.notify();
        return;
    }
    view.fly_to_region(&drag.region, window, cx);
}
