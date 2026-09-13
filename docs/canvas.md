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
| Tool armed, click | `PlaceNode` with the default size centred on the cursor |
| Tool armed, drag ≥ 4 px | `PlaceNode` with the dragged rect, clamped to `min_size` |
| Camera locked | pan/zoom effects dropped; selection and drags still work |
| Gesture end | `GestureEnded` → commit viewport |

The gpui side (`peek-ui/src/canvas/mod.rs`) only translates events into `Input`, applies
`Effect`s to the camera or the `Document`, and notifies.

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
- the top `HEADER_WORLD_HEIGHT` (32, the shell's `rems(2.0)` at base rem 16) is the drag handle.
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
escape clears it through the existing `Tool::Select` handler. Unlike the TypeScript app, which
creates the node on drag start and mutates it every frame, the rect is computed in the reducer
and the node is created **once** on mouse-up — same look through the same overlay the marquee
uses, but no undo or autosave churn and nothing to clean up when a placement is cancelled.

## Node zoom strategy

gpui has no transform groups (zed-industries/zed#53303), so nodes cannot be scaled as a subtree.
Instead each node is a **real gpui element tree** placed at camera-derived screen coordinates and
laid out inside `window.with_rem_size(Some(base_rem * zoom))`. Everything in the shell is
rem-based, so text, padding and radii scale with the camera while borders stay one physical
pixel. Hit-testing works unchanged because element hitboxes are already in screen space.
Zed's `crates/ui/src/utils/with_rem_size.rs` is the precedent.

Nodes render as full element trees at every zoom, and so does the grid: the placeholder level of
detail that painted flat quads below zoom 0.25 was a limit of the browser renderer, not of gpui,
and culling against the visible rect already bounds the work. `grid::grid_step` doubles the world
gap until dots are ≥ 12 px apart, so the grid thins out instead of turning into noise.

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

**Overlays lay out outside the rem scope.** Anything gpui renders in the window overlay —
popovers, tooltips, menus — is laid out at rem 1 whatever the camera is doing, because the rem
scope only wraps the node's own element tree. A popover is therefore not usable as part of a
node's UI (the Variable node expands its list editor inline instead); a tooltip is defensible,
being chrome *about* the node rather than part of it, but it will not scale with the node it
describes. Where a tooltip carries information, put the same text in an `aria_label` so dropping
it later costs nothing.

Escape is worth knowing about too: it is `Tool::Select`, bound on `"Canvas"` even while typing,
and gpui dispatches bound actions *before* any key listener, capture phase included — so no node
can intercept it with `on_key_down` or even `capture_key_down`. Handle the action itself on the
card, which sits below the canvas on the focus path, and `stop_propagation`; a second escape then
reaches the canvas and clears the selection.

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

## HUD

Bottom-left cluster: `−`, zoom %, `+`, `Fit`, `Lock`. Every button dispatches the same action
the keyboard shortcut does through the canvas focus handle.
