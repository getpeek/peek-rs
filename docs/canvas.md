# Canvas, camera and rendering

## Camera

`peek_canvas::Camera { pan, zoom }` mirrors React Flow's viewport: `screen = world * zoom + pan`,
persisted as `Page.viewport { x, y, zoom }`. Zoom is clamped to `0.1..=4.0`.

Key operations (all pure, unit-tested in `camera.rs`):

- `zoomed_about(anchor_screen, new_zoom)` keeps the world point under the cursor fixed.
- `centered_on(world, zoom, pane)` is React Flow's `setCenter`.
- `fit_bounds(bounds, pane, FitOptions { padding, min_zoom, max_zoom })` is `getViewportForBounds`.
  Peek always caps fit at `max_zoom: 1`.
- `visible_world_rect(pane)` drives culling.

## Fitting

Three commands say "fit" and only two of them are camera moves.

- `Zoom::FitView` (`cmd-shift-0`) frames every node on the page — `content_bounds` through
  `fit_bounds`, capped at 100%.
- `Zoom::FitSelection` ("Fit nodes to view") is **a layout, not a camera move**: it tiles the
  selected nodes across the pane and then puts the camera at exactly 100% over the point it was
  already looking at. It is the only zoom command that writes to the document.
- `Zoom::FitSelectionAndLock` is that, then the camera lock — set, never toggled.

The tiling is `peek_canvas::layout::bsp`, a port of `bspTiles.ts`: recursively split the region
along its longer axis, `ceil(count / 2)` tiles to the first half, one `FIT_GAP` (16) out of the
axis per split and `FIT_PADDING` (24) around the whole layout. Tiles are handed out in **page
order**, matching `rf.getNodes()` — not the sorted order `Document::selected` keeps ids in.

Two deliberate divergences from the reference. Tiles are clamped to `NodeType::min_size()`,
because `set_bounds` clamps and a twenty-node selection is better as overlapping readable cards
than as slivers below their own resize minimums. And `title_bar::HEIGHT` stands in for the
hardcoded `TITLEBAR_HEIGHT = 50`, so a fit with the chrome hidden uses the whole window.

"Screen" in the camera is **pane space** (the canvas element's own coordinates). The view stores
the pane's window bounds (`pane_bounds`, written by the element every prepaint) and subtracts the
origin from window-space pointer positions before feeding the reducer.

## Flights

`CameraFlight` interpolates the **world centre linearly and the zoom geometrically** (log space)
with ease-in-out cubic, so the focal point never drifts sideways. Durations come from the TS app
(`flight::durations`: 300 ms fit/pan, 200 ms reset, 150 ms zoom buttons, 250 ms fit button,
600 ms region fly). Flights are ticked in `CanvasView::render` on wall-clock time and request the
next animation frame only while active; `cx.reduce_motion()` makes them instant. A van Wijk / d3
`interpolateZoom` variant is the intended upgrade for the long region fly.

## Viewport persistence

The viewport is written to the document in exactly one place, `CanvasView::commit_viewport`,
called on gesture end (mouse up, pinch end, trackpad wheel `TouchPhase::Ended`), flight end, the
140 ms quiet period after a phase-less mouse wheel, and page switch. Never per frame.

## Gestures

`peek_canvas::gesture::reduce(state, config, camera, selected, input) -> Vec<Effect>` is the
whole pointer model, tested per row:

| Input | Effect |
|---|---|
| Trackpad scroll (`ScrollDelta::Pixels`) | `Pan(delta)` — gpui's delta is already the content's movement direction |
| Mouse wheel (`ScrollDelta::Lines`) | `Pan(lines × 20 px)` |
| cmd/ctrl + wheel | `ZoomAbout { factor: 2^(-dy × 0.01) }`, clamped to `[0.5, 2]` per event |
| Pinch (`PinchEvent.delta`) | `ZoomAbout { factor: 1 + delta }` |
| Middle or right drag, space + left drag | `Panning` |
| Left down on empty canvas, moved ≥ 4 px | `Marquee` (partial overlap, shift extends from the prior selection) |
| Left down on empty canvas, up < 4 px | `DeselectAll` (no-op with shift) |
| Left down on a node header, up < 4 px | `SelectOnly` / `ToggleSelect` with shift |
| Left down on a node header, moved ≥ 4 px | `DraggingNodes` of the selection (an unselected node is selected first) |
| Left down on a node body | `SelectOnly` on release; never a drag and never a marquee |
| Left down on a node's resize zone | `ResizingNode` → `ResizeNode { id, bounds }` per move |
| Tool armed, click | `PlaceNode` with the default size centred on the cursor, then `CommitPlacement` |
| Tool armed, drag ≥ 4 px | `PlaceNode` with the dragged rect on the first frame past the threshold, `ResizePlacement` per move after it, `CommitPlacement` on release |
| Pen armed, left drag | a sample per move in world units; `PlaceDrawing` on release from two samples up |
| Pen armed, any other button | nothing: the pen owns the pointer, and wheel and pinch still pan and zoom |
| Camera locked | pan/zoom effects dropped; selection and drags still work |
| Gesture end | `GestureEnded` → commit viewport |

The gpui side (`peek-ui/src/canvas/mod.rs`) only translates events into `Input`, applies
`Effect`s to the camera or the `Document`, and notifies.

## The draw tool

`Tool::Draw` (`d`, and the toolbar's pencil) arms the pen. It is the only **sticky** tool:
`useDrawTool.ts` never clears place mode, so a committed stroke leaves it armed for the next
one until escape — every other place tool is one-shot.

While the pen is armed it owns the pointer, which is what the reference's `stopPropagation`
buys: no press reaches the pan, marquee, drag or resize paths, so a stroke started over a node
draws instead of moving it. `gesture::on_drawing` is offered every input before the ordinary
reducer and hands back wheel and pinch, so two-finger scroll and pinch zoom keep working.

Three deltas from the reference, each recorded where it is made:

- **Samples are world units, converted on arrival.** The reference buffers client coordinates
  and converts the batch at commit, which is only equivalent because its camera cannot move
  mid-stroke. Converting here keeps the camera off the commit path, so a pan during a stroke
  moves the ink with the canvas instead of shearing it.
- **No pressure is recorded.** gpui's `MouseMoveEvent` carries none, and the reference stores
  `e.pressure || 0.5` — a constant for every mouse stroke. `simulate_pressure` derives width
  from sample spacing, so the stored `0.5` is never read back.
- **A non-left press does nothing.** The reference returns before its `stopPropagation`, so
  React Flow's `panOnDrag={[1, 2]}` still pans mid-mode; matching that would mean teaching
  `Panning` to return to `Drawing` on release. The loss is middle-drag, right-drag and
  space-drag until escape.

`Document::create_drawing` is the commit, and it is one undo step: the node and its data go in
one `transaction`, so undoing a stroke never leaves an empty drawing behind. The geometry is
the reference's — the samples' extent inset by `PADDING = stroke_width * 2 = 8` on every side,
points stored relative to the node's origin. The colour is set explicitly to `var(--pk-fg)`,
because `NodeKind::empty` yields `makeNode`'s palette default of `white` and every stroke would
otherwise be invisible in a light theme.

The live preview is `LiveStroke.tsx`: pane-relative screen points painted above the nodes, at
`stroke_width * 4 * zoom`, through the same `node::draw::tessellate` the committed node uses so
the shape cannot change at the moment the pen lifts. It shares `peek_canvas`'s `DRAW_COLOR` and
`DRAW_STROKE_WIDTH` with the commit for the same reason.

## Node hit regions

Which part of a node was pressed is decided in **world space** by `peek_canvas::hit::node_hit_at`,
never by listeners on the node element. `CanvasElement::paint` registers its window-level mouse
listeners last, and gpui dispatches the bubble phase in reverse registration order, so the canvas
always hears a press before the node under it does; resolving the region from the pointer's world
position instead keeps the whole rule in one pure, unit-tested function.

The header band is constant in world units, because the shell is rem-based inside a rem scope of
`base_rem * zoom` — so a header is the same 32 world units tall at every zoom. The resize bands
are the exception: they are **screen**-space, converted to world units with the camera's zoom, so
the grab keeps its width under the pointer instead of thinning out as the camera pulls back.

- `RESIZE_EDGE_SCREEN_INSET` (12 px) from any edge is a resize grab, and `RESIZE_CORNER_SCREEN_INSET`
  (18 px) from two edges at once is a corner grab — a corner is the harder target, so it claims the
  wider band and wins over the side it sits in. Resize wins over the header, so the top corners stay
  grabbable and the header's own top edge resizes.
- Two caps keep the band honest: it may grow to at most `MAX_ZOOM_OUT_GROWTH` (2×) its screen inset
  in world units as the camera zooms out, and it is never more than a third of either axis. Without
  the first, a card at zoom 0.1 would be resize band edge to edge with nothing left to drag.
- the top `HEADER_WORLD_HEIGHT` (36, `.app-node-header`'s `min-height` at base rem 16) is the drag handle.
- everything else is the body, which selects on click and drags only while the secondary modifier
  (cmd) is held at the press: cmd turns the whole card into a drag handle, moving the selection if
  the pressed node is part of it and selecting it first if not. An edge press never drags, modifier
  or not.

Draw nodes are skipped entirely: `useDrawTool.ts` commits them with `pointerEvents: "none"`, so
a stroke is decoration rather than a card — no drag handle, no resize corners, and presses fall
through to whatever is beneath. `nodes_in_rect` still includes them, so a marquee selects them:
pointer transparency is not invisibility.

Resizing from a top or left edge moves the origin, so `Effect::ResizeNode` carries a whole
`Rect` and the view calls `Document::set_bounds`, which clamps to `NodeType::min_size`. The
press anchor and every later pointer position are converted to world units before they are
subtracted, so the bounds follow the pointer whatever the camera's pan and zoom.

The same regions drive the cursor: an idle move re-reads the region under the pointer, so a
resize zone shows its diagonal and a header shows the grab hand (`node.css` gives the header
`cursor: grab`) before the press. The body sets no cursor of its own.

## Edges

`peek_canvas::edge` is the port of `FloatingEdge.tsx` and `floatingEdgeUtils.ts`. Peek's edges
have no handles: both endpoints are recomputed every frame as the point where the line between
the two node centres crosses that node's box, and the side it crosses decides which way the
curve leaves. Control points follow React Flow's `getBezierPath` at curvature `0.25`, with the
square-root splay for overlapping nodes so the curve still bulges when the gap goes negative.

Painting and hit-testing share `curve_between`, so what you press is what you see.
`peek-ui/src/canvas/edges.rs` strokes the cubic through `PathBuilder::stroke`; like every other
raw-pixel path it scales its own width by the zoom, since the rem scope does not reach path
coordinates. Colour comes from `PeekTheme::edge`, tinted by the kind the edge **points at** —
"what this feeds" — with the three `node.css` states: resting (dimmed), selected (full opacity,
same tint) and `.connection-active` (either node selected: thicker, and mixed toward white).
The last wins where both apply, matching the stylesheet's order.

`hit::edge_hit_at` walks the page's edges in reverse — paint order is array order — rejects on
the control hull, then flattens the curve and takes the minimum distance to a chord.
`EDGE_HIT_WORLD_WIDTH` is 20 **world** units, not screen: React Flow's transparent interaction
stroke lives inside the transformed viewport, so the target grows on screen with the zoom.
`hit::hit_at` composes it after `node_hit_at`, so a card always wins over a curve beneath it.

Selection is a second `BTreeSet<EdgeId>` on the session `Document`, not the `Edge.selected`
field. `history::Snapshot` holds `Vec<Edge>` and compares by value, so a selection stored on the
edge would turn selecting into an undoable edit and make undo restore an old selection. Picking
an edge clears the node selection and picking a node clears the edge selection, as React Flow's
store does; a marquee therefore clears it too. `Document::remove` deletes nodes and edges in one
`EditKind::Structure` transaction, because that kind never coalesces and two calls would be two
undo steps for one Delete.

Pressing an edge and dragging is inert, the same rule a body press follows: `useRubberBandSelect`
refuses to start on an edge and React Flow has no edge drag, so the press stays pending and the
release still selects.

## Placement

`Input::ArmTool(Some(kind))` puts the reducer in `Interaction::Placing` (crosshair cursor);
escape clears it through the existing `Tool::Select` handler. As in the TypeScript app
(`usePlaceTool.ts`), the node is created on the first move past the drag threshold and resized
every frame after it, so what the drag sizes is the node itself rather than a preview rectangle;
a click without a drag places a default-size node centred on the cursor. The release
(`Effect::CommitPlacement`) frames the node at 100 % and hands a query node straight to its SQL
editor.

The reducer stays pure: it emits `PlaceNode`, then `ResizePlacement`, then `CommitPlacement`, and
the canvas view holds the `NodeId` the document minted. The whole drag is **one undo step** —
`Document::create_node` opens an `EditKind::Structure` transaction and `resize_placement` opens
none, so the checkpoint at the release seals the lot. Escape mid-drag calls
`Document::cancel_placement`, which removes the node and discards that transaction, leaving
nothing to undo.

## Node zoom strategy

gpui has no transform groups (zed-industries/zed#53303), so nodes cannot be scaled as a subtree.
Instead each node is a **real gpui element tree** placed at camera-derived screen coordinates and
laid out inside `window.with_rem_size(Some(base_rem * scale))`. Everything in the shell is
rem-based, so text, padding and radii scale with the camera while borders stay one physical
pixel. Hit-testing works unchanged because element hitboxes are already in screen space.
Zed's `crates/ui/src/utils/with_rem_size.rs` is the precedent.

**A node's box uses the camera's zoom; its contents use a snapped `scale`.** `peek_canvas::
render_scale` rounds the zoom to a ladder of twelve rungs per octave, anchored on 1.0, and that
is what feeds both the rem scope and `NodeContext::zoom`. gpui keys its line-layout cache and
its glyph rasteriser on the exact font size (`RenderGlyphParams`, `line_layout::CacheKey`), so a
zoom that moves continuously misses both caches for every visible string on every frame and
re-shapes the whole viewport; the cache for rasterised bounds is not even bounded. Snapping
turns that into a hit on every frame that does not cross a rung. The cost is that type inside a
card can sit up to ~3 % off the card around it, which is below what the eye resolves — and
positions, edges, hit testing and the selection ring all still use the exact zoom, so nothing
drifts relative to anything else.

**Under `--performance`, below `lod::REDUCE_BELOW` (0.32) a node draws its shell and no body**, coming back at
`RESTORE_ABOVE` (0.38); the gap is hysteresis, so a slow zoom crosses once instead of rebuilding
every visible body twice a frame. Culling bounds the work only while nodes leave the viewport,
and zooming out does the reverse — more cards fit the screen the further back the camera goes —
so past the point where a body is readable, building it is pure cost. Exempt: a selected node,
the bare kinds (Text and Draw have no shell, so reducing them would make them vanish), and every
node while focus sits anywhere but the canvas, which means an editor owns it. A reduced node is
also not allowed to *create* retained state, so zooming out over a page of query nodes does not
open a language-server document for each one. The reference draws the same line at 0.35
(`wayfinding/crossFade.ts`), where it stops dimming nodes and hands the board to region beacons.

Without the flag none of that happens: `CanvasView::resolved_detail` returns `Detail::Full`
before it looks at the camera at all, and every node builds its real body at every zoom. The
trade the tier makes is real — a page of shells is harder to recognise than a page of nodes —
so it is the user's to ask for, not the default.

The grid thins the same way: `grid::grid_step` doubles the world gap until dots are ≥ 12 px
apart, so it fades out instead of turning into noise.

**The rem scope scales rem-based sizes, not raw geometry.** Anything painted in pixels inside a
node — a `gpui::Path`, a quad, a `canvas()` element — is unaffected by `with_rem_size` and must
scale itself, recovering the factor as `bounds.size.width / node.size().width`. The Draw node
does this; it is the reason a stroke's thickness tracks the camera at all.

Two traps in the same area, both found the hard way:

- `Style::paint` applies no content mask — `Div` does its own clipping — so `.overflow_hidden()`
  on a `canvas()` element compiles, looks right and clips nothing. Clip with an explicit
  `window.with_content_mask`.
- lyon's `FillOptions` defaults to even-odd while SVG defaults to nonzero. A self-intersecting
  outline (any freehand stroke with a sharp corner) gets holes punched through the overlaps
  unless the builder sets `FillRule::NonZero`.

## Frame flow

1. `CanvasView::render` ticks the flight, culls nodes against `visible_world_rect` dilated by
   64 px, and builds a `NodeItem { world, AnyElement }` for every visible node.
   Elements must be built here because `Element::prepaint/paint` only get `&mut App`.
2. `CanvasElement::request_layout` requests a full-size layout with no taffy children.
3. `prepaint` records the pane bounds on the view (notifying only on change), inserts one
   hitbox, and for each element item calls `layout_as_root(Definite(w×zoom), Definite(h×zoom))`
   then `prepaint_at(origin)` inside the rem scope.
4. `paint` order: base fill → vertical gradient → dot grid → edges → node elements
   (rem scope again) → selection rings (constant 1.5 px, 3 px offset) → marquee. Then it registers
   **window-level** mouse listeners (`window.on_mouse_event`) for down/move/up/wheel/pinch so pans
   and marquees keep receiving events across node hitboxes; the view gates them on its
   `Interaction` state. Node hitboxes are ordinary (`HitboxBehavior::Normal`), so the canvas
   hitbox is still hovered over a node and a press reaches `mouse_down`; the wheel listener is
   gated on `should_handle_scroll`, which an ordinary hitbox permits, so wheel over a node still
   pans (the old `useScrollFallthrough` behaviour).

   **Wheel and pinch are registered first instead**, at the top of `paint`, before the node
   elements are painted. Reverse bubble order then offers a wheel to the hovered node before
   the canvas, which is what lets a node body scroll: gpui-base's editor calls
   `stop_propagation` only when its scroll offset actually changed, so an already-scrollable
   body absorbs the wheel and everything else falls through and pans. That is
   `useScrollFallthrough`'s `canAbsorb` rule, already written, in the editor.

   cmd/ctrl + wheel is the exception and is handled in the **capture** phase, where the canvas
   is first: zooming stops propagation so an editor under the pointer never swallows it. The
   reference bails out of absorption on `ctrlKey` for the same reason.

   A kind built on a plain `overflow_y_scroll` div does not get this for free — gpui's built-in
   scroll listener does not stop propagation — so it must add its own `on_scroll_wheel` that
   stops only when it consumed. The Result node's table is the worked example
   (`node/result/has_room_to_scroll`): `DataTable` adds no wheel handler of its own, so without
   one a scroll would move the rows *and* pan the canvas.

## Node shell

`NodeShell` (`RenderOnce`) draws the header (kind indicator as dot or tick, uppercase kind label,
title, and optional per-kind controls) around a body element the kind supplies. Kinds fan out in
exactly one place, `node/kind.rs`, into one folder each under `node/`; `node/state.rs` holds the
retained state for kinds that have an editor or a scroll position, keyed by node id and pruned
against the document rather than against visibility. Text and Draw are *bare*: they draw their
own card with no shell chrome, matching `TextNode.tsx` and `DrawNode.tsx`. Terminal and Blueprint add eight
absolutely positioned corner-bracket slivers. Selected nodes get the strong border; the ring is
painted by the canvas overlay so it can keep a constant screen width. Colours come exclusively
from `cx.peek_theme()`.

## Editable nodes

Two rules every kind with an editor has to follow. Both were learned by breaking them.

**The card is not focusable.** `track_focus` on a node's root makes the element focusable, so a
click lands focus on the *node* rather than the canvas — and when that node is deleted its handle
dies with it and the window is left focused on nothing, which silently kills every canvas
binding including `cmd-z`. Leave the canvas as the nearest focusable ancestor, and have the
editor stash whatever held focus when it opened and hand it back on Enter or Escape.

A body that legitimately owns a focus handle — the SQL editor, the results table — cannot avoid
this, so `CanvasView::reclaim_focus` is the backstop: after pruning node state each frame, a
window left focused on nothing is handed back to the canvas. That covers deletion by every
route, not just the Delete key.

**A node body still receives its own presses.** The canvas hears a press first and resolves it in
world space, but it does not `stop_propagation`, so the node's own handlers run straight after —
a click on a table cell selects that cell *and* the node. The world-space hit test decides what
the *canvas* does with the press (drag the card, marquee, select), not whether the node sees it.

**Reconcile against the revision, not every frame.** A keystroke reaches the input before the
change event that carries it to the document, and a frame can render in between. A view that
re-adopts the document's text unconditionally will push the older value back over what was just
typed. Gate the re-adoption on `Document::revision()` changing.

**A component's own callbacks run inside its update, so they must not reach back into it.**
`DataTable` builds its cells through the delegate, so a handler attached in `render_td` fires
while `TableState` is mid-update. Calling anything that *reads that state back* from there — the
Result node's editor did, to fetch the draft and the editable table — is a re-entrant borrow, and
gpui turns it into a **non-unwinding panic**: the process aborts, taking the window with it. It
is not a crash the user can dismiss.

The escape hatch is `window.defer`, which runs the work on the next turn, once the update has
finished; the Text node already uses it for its auto-grow. The test that pins it
(`double_clicking_a_cell_opens_an_editor_without_re_entering_the_table`) needs a connection,
because every editing path bails before the read without one — which is why the hermetic suite
could not have caught it first time, and why `Database::mark_connected_for_test` exists.

**Where an overlay lays out depends on where it was raised, and the old note here had it
backwards.** `Window::defer_draw` captures `rem_size` and both deferred passes re-enter
`with_rem_size`, so a `deferred` draw raised from **inside** a node body inherits
`BASE_REM * zoom` and scales with the camera — `node/query/mod.rs` found that by having to
hand-scale the completion popover's `max_width`. One raised **outside** the node tree, as a
sibling of `CanvasElement`, is rem 1 and does not scale; that is what the jump badges, the
page-search panel and the right-click menu are.

It is also **not** clipped by the node body's `overflow_hidden`: `deferred()` passes
`content_mask: None` and `with_content_mask(None)` is a no-op in gpui-pre 0.3.4. The earlier
claim that it was is why the value pane and the Variable list editor are inline; that conclusion
still stands, but on the scaling argument alone.

So the question to ask is not "can it be an overlay" but **is it content or chrome**. A pane
explaining a cell belongs to its node and should scale with it, so it goes inline. A context
menu, a tooltip, a tool palette does not — the palette and the zoom cluster are already pinned in
pixels — so it goes on the canvas, outside the rem scope. Where a tooltip carries information,
put the same text in an `aria_label` so dropping it later costs nothing.

**Right-click is the node's to claim.** The canvas parks a right press in `PendingPress` and
`on_up` bails for any non-left button, so a stationary right-click does nothing to it; only a
right *drag* pans. But `DataTable` attaches its own `ContextMenu` to the element wrapping every
row and registers that window listener **after** painting its children, so it is dispatched
*before* them — a cell's own handler can neither beat it nor `stop_propagation` it. The Result
node takes the press on a transparent sibling painted after the table, which registers later
still; see `node/result/mod.rs`, `right_press_catcher`.

Escape is worth knowing about too: it is `Tool::Select`, bound on `"Canvas"` even while typing,
and gpui dispatches bound actions *before* any key listener, capture phase included — so no node
can intercept it with `on_key_down` or even `capture_key_down`. Handle the action itself on the
card, which sits below the canvas on the focus path, and `stop_propagation`; a second escape then
reaches the canvas and clears the selection.

## Regions and wayfinding

`peek_canvas::regions` and `peek-ui/src/canvas/wayfinding/`, the port of
`~/labs/peek/src/canvas/wayfinding/`. The thing to know first: **a region is not a drawn
rectangle.** It is a named, coloured *set of node ids* with no geometry of its own
(`Region { id, name, desc, colorIndex, status, memberIds }`, frozen on disk). Its box is
re-derived every frame from the members that still exist, inflated by `REGION_PADDING` (56).
So there is no drag-to-draw, no resize, no z-order, and no spatial containment: moving a node
under a region's box does not join it, and the only way to move a region is to drag its beacon,
which translates the members.

`member_ids` is allowed to name deleted nodes — `regions::derive` filters them out, which is
what keeps deletion and undo simple — but a region whose members are *all* gone is dropped, by
`Document::prune_empty_regions` inside the same transaction the removal opened. That is
`state.ts:pruneEmptyRegions`, and it is why deleting a region's last node and the region itself
are one undo step.

**Membership is exclusive.** Grouping claims nodes from whatever region held them, and a region
the claim empties goes with them. `Document::group_plan` is the whole of ⌘G's decision, pure and
unit-tested: fewer than two nodes selected, or a selection that is already exactly one region's
members, is `Unavailable`; a selection touching exactly one region is `FoldInto` (the region
grows and keeps its name); anything else is `Create`.

### The cross-fade

`regions::crossfade` is `crossFade.ts`: `t = clamp01((0.35 - zoom) / 0.14)`, so the camera is
reading at `t = 0` and navigating at `t = 1`. Everything reads it — the confirmed halos take it
as their opacity, beacons fade in with it and become pointer-interactive past 0.35, peekers fade
out at `1 - t * 1.4`, and past `DIM_THRESHOLD_T` (0.4) the nodes drop to 0.42 and the edges to
0.35. `lod.rs` draws its own line a little earlier (0.32) and for a different reason: it stops
*building* bodies rather than dimming them.

### Halos are painted over the nodes

The one world-space surface. `CanvasElement`'s paint order is
`grid → edges → node elements → halos → selection rings → stroke → marquee`, and the halos being
**above** the cards is load-bearing rather than incidental: React Flow's `ViewportPortal` mounts
into `.react-flow__viewport-portal`, the last child of the viewport, and a confirmed halo is a
veil of the *canvas background colour* — painted underneath the cards it would do nothing at all.
Every halo is pointer-transparent, so hit testing is untouched.

gpui has no radial gradient (`Background` is solid, linear, slash or checkerboard), so the CSS's
`radial-gradient(ellipse 80% 80% …)` is rebuilt as twelve concentric rounded quads scaled about
the box's centre, each adding the *difference* between neighbouring stops rather than the stop
itself. Scaled rather than inset, because a uniform inset collapses the inner bands of a wide
region to nothing. A confirmed halo is only painted at all once `t > 0`, so at working zoom it
costs nothing.

Suggested regions are different: a dashed 1.5 px box in the region's colour over a 4 % fill,
visible at *every* zoom, because it is asking for a decision. The flash ring that marks a fold
is its own box for the same reason the suggested one is always visible — a confirmed halo is
transparent at the zoom where folding happens — and `CanvasView::tick_flash` requests the frames
it fades over, since nothing else on the canvas is moving when one appears.

### Beacons, peekers and the review card are chrome

All three are late children of `CanvasView`, outside the rem scope, so they keep a constant
screen size at every zoom — a way of *reaching* a region is not part of it, and the review card
is a surface you act through. The reference has to counter-scale its card by `1 / zoom` to get
the same thing, because its card lives inside the zoomed viewport.

A beacon owns its press: the canvas registers its pointer listeners before this element paints,
so gpui's reverse bubble order offers the press here first and stopping it is what keeps a press
on a beacon from also starting a marquee. The release, though, does **not** arrive here — the
press repaints, and the frame it produces carries the full-window drag catcher, which occludes
the beacon. So click-versus-drag is decided in `beacons::end_drag`, off the region id the press
recorded; an `on_click` could not do it either, because the click fires after the release has
already cleared the state it would have to read.

Peekers are a transient compass rather than a permanent one: `Wayfinding::nudged_at` is re-armed
by every camera change and by the pointer resting on a label, and they fade 900 ms after the last
nudge. The arrow is one of eight compass glyphs rather than the reference's rotated SVG triangle,
because gpui has no self-relative rotation on a div.

`useCanvasWheelForward.ts` needs no port: the canvas registers its wheel listener at the top of
`paint` and these overlays paint later, so a wheel over a beacon already reaches the canvas.

### The picker

`wayfinding/menu.rs`, a 280 px panel above the zoom cluster, hand-owned for the reasons
`title_bar/pages/panel.rs` records — a field inside a `Popover` never sees a space, and the press
that dismisses one has to be swallowed before it reaches the canvas. It is the only way to reach
a region by name, and the only place a region created by ⌘G gets one: `Region::GroupSelection`
opens it with the new region in rename mode rather than inventing a name and leaving it.

Two re-entrancy rules the panel had to learn, both the same shape. `open_renaming` and `close`
are called from inside a `CanvasView` listener, where that entity is leased — so neither may read
the canvas back. The cursor is moved to the renaming row in `render` instead, and focus is handed
back through a `restore_focus` handle captured on open rather than by asking the canvas for one.

The panel's two sparkle buttons — one on the header, one on the `Ungrouped` row — dispatch
`Region::RegroupAllWithAi` and `Region::GroupWithAi` through the canvas focus handle, so a
button, a palette row and a future keybinding are all the same command. They exist only when
`ai.ollama` is configured.

### Letting the model group

`peek_canvas::regions::grouping`, the port of `useAiGrouping.ts`, `useGroupWithAi.ts`,
`useRegroupAllWithAi.ts` and `clusterUngrouped.ts`. **The model decides the grouping, not just
the names.** A `Prompt` is built from the page — numbered node lines (`[3] query [120,-40]
Active users — select …`), the edges between them, and, for the living-document variant, the
existing regions as `[R#]` anchors — the caller sends it wherever it likes, and `parse` turns
the reply back into a `GroupingPlan`. Both system prompts are the reference's verbatim: the two
apps read the same documents, so the same page should come back grouped the same way.

The two commands differ only in what the prompt asks and what the plan does with the answer.
`Prompt::extend` shows the model the ungrouped nodes and the regions they could join, and its
plan *adds*; `Prompt::partition` shows it everything, and its plan *replaces*. Which one a plan
is cannot be chosen by a caller — the field is private, set by the prompt that produced it.

Everything here is pure, and the whole plan lands through `Document::apply_grouping` in one
transaction: a grouping is reviewed as a whole, so undoing it is one ⌘Z however many regions it
touched. Every region it creates is `Suggested`, which is what the Keep / Rename / Dismiss card
over each one is for.

A model that cannot be reached, or that answers with prose, is **not** an error:
`Prompt::fallback` clusters the same nodes geometrically — union-find over the edges between
them plus a 420 px proximity radius — and names the clusters `Group N`. The reference falls back
the same way, and it is why the feature degrades to something useful rather than to nothing.
The describing half is shared with page search (`peek_canvas::describe`), so a node kind is
described once for both, and a query labelled by the model reaches the prompt as its title.

## Keyboard navigation

Three ways to move between nodes without the mouse, all ported from the reference. The maths and
the state machines are in `peek-canvas` (`jump.rs`, `direction.rs`); `peek-ui` only dispatches
and draws.

**`g` — jump labels** (`src/canvas/jump/`). Every node intersecting the viewport gets a badge
carrying a short code from the home-row-first alphabet `asdfghjklqwertyuiopzxcvbnm`, nearest to
the viewport centre first, so the easiest keys land where the user is already looking. Labels are
**uniform length** — one character up to 26 nodes, two beyond — which is what stops any label
from being a prefix of another and lets a complete code fire the moment it is typed. Typing a
letter that matches nothing is *ignored* rather than treated as a cancel, so a mistype costs
nothing; backspace shortens the prefix and never exits. Picking a node selects it alone and flies
to it at **zoom 1** over 300 ms. Non-matching badges stay on screen, faded: the set visibly
narrows instead of blinking away.

The overlay is a scrim plus absolutely positioned badges, rendered as the last child of
`CanvasView` so it dims the HUD and toolbar too. Badges sit **outside** the rem scope and so keep
a constant screen size at every zoom — a label is a way of reaching a node, not part of it. gpui
has no self-relative transform on a div, so the reference's `translate(-35%, -35%)` anchor is
approximated with a fixed pixel nudge; the badge's width follows its label, so the two cannot
match exactly.

How jump mode takes the keyboard is a key-context trick rather than a second focus handle — see
`docs/commands.md`, "Key contexts".

One divergence follows from that trick. `!JumpMode` kills *every* canvas binding while labels are
up, not only the bare-key ones, so a modifier combo bound on the canvas — `cmd-=`, `cmd-a` — is
spent cancelling jump mode rather than doing its job; press it again and it works. The reference
cancels and runs the combo in one go, because its overlay is a capture-phase listener that can
decline to consume. The rule here is simply "any modifier cancels the jump", which is at least
consistent; combos bound above the canvas (`cmd-p`, `cmd-q`) still fire on the first press, and
jump mode drops itself when the canvas loses focus.

**`cmd-arrow` — directional selection** (`usePageActions.ts`). Candidates must lie inside a 45°
cone ahead of the current node's centre (exactly 45° counts), and the winner is the lowest
`reach + 2 × drift`: off-axis drift costs double, so a node squarely in the pressed direction
beats a nearer skewed one. Ties go to whichever node comes first in the document. With nothing
selected the first press ignores the direction entirely and anchors on the node nearest the
viewport centre. Unlike jump mode this considers **every** node on the page, not just the visible
ones, and the flight **keeps the current zoom**.

**`enter` — edit the selected query.** Exactly one node selected, and it a Query: focus its SQL
editor. Escape hands focus back to the canvas, and a second escape clears the selection. The
editor remembers what held focus when it took it — but only a *click* reaches that handler before
the window's focus actually moves. `Query::Focus` moves focus first, so `QueryState::focus_editor`
stashes the outgoing handle itself and the editor's `InputEvent::Focus` arm declines to overwrite
it with the editor's own handle. Without that guard escape is a silent no-op.

## Chrome

The title bar is an **overlay**: the canvas fills the window and the bar floats over it, as the
reference does. Two consequences worth knowing before touching `WorkspaceView::render`:

- The bar must be a **later child than the canvas** and carry `occlude()`. Ordinary hitboxes do
  not block each other — that is what lets a wheel over a node still pan — so without
  `occlude()` (`HitboxBehavior::BlockMouse`) a click on a page tab also reaches the canvas'
  window-level listeners and starts a marquee.
- `CanvasView::pane_size` subtracts the chrome inset, so `Zoom::FitView` frames content in the
  visible region rather than centring it behind the bar.

macOS draws the traffic lights; suppressing them would need `unsafe` and an AppKit dependency,
both of which `CLAUDE.md` forbids, and `window_control_area(Close|Min|Max)` is an empty function
on this platform. `title_bar::window_options()` positions them for our bar height instead.

There is no `backdrop-filter` in gpui — its only blur is `BoxShadow::blur_radius` — so the bar
occludes what is behind it rather than frosting it.

`View::ToggleUi` hides every chrome surface, not just the bar: `CanvasView::chrome_visible`
gates the HUD and the toolbar, and feeds `Scope::chrome_hidden`.

### Chrome or content: where a surface raised by a node belongs

Three of them are raised by nodes and drawn by the canvas — the right-click menu
(`canvas/context_menu.rs`), the jump scrim (`canvas/jump.rs`) and the JSON editor
(`canvas/json_editor.rs`) — while the Result node's value pane and the Variable node's list
editor stay inside the node. The dividing line is **not** technical. A `deferred` draw raised
from inside a node body inherits `BASE_REM * zoom` and is *not* clipped by the body's
`overflow_hidden`, so either is buildable either way; `node/result/detail.rs` records how that
was got wrong in both directions before anyone checked.

The question is what the surface *is*:

- A surface **explaining** a cell is content. It belongs to its node, should grow with the
  camera, and reads better pushing the table down than floating over it.
- A surface you **act through** is chrome. A menu that doubles in size when you zoom in is
  chrome behaving like content; a field you type into at zoom 0.4 renders four-pixel text, and
  the document you are editing is the one thing on screen that has to stay legible.

So the value pane and the JSON editor sit on opposite sides of the line for the same cell, and
both are right. The editor pays for it by having to be told where its cell is: the cell reports
its own bounds from `on_prepaint` every frame the panel is open, and the canvas moves the panel
only when they actually change — a repaint per report would request the next frame that produced
the next report.

## HUD

Bottom-left cluster: `−`, zoom %, `+`, `Fit`, `Lock`, and — behind `--fps` — a frame-rate
readout. Every button dispatches the same action the keyboard shortcut does through the canvas
focus handle.

The readout is a segment inside the same pill rather than a pill of its own, because an
absolutely positioned sibling cannot know where the first one ends. It is **passive**: it reads
the frames `FrameStats::tick` already counts, and never asks for one of its own — a counter that
drives frames measures itself. gpui redraws on demand, so a still canvas produces no frames and
the honest reading is `idle`, not zero. The single concession to the frame loop is a settle
timer, re-armed each frame and dropped by the next, so the last frame of a gesture leaves exactly
one trailing repaint behind to show `idle`.
