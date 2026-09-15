# Decisions and rationale

Decisions taken with the user, plus reasoning that is not visible in the code.

## Confirmed with the user

- **Real `~/peek`, read-only for now.** The rewrite reads the same settings and documents the
  Tauri app uses, and never writes until the mutation milestone. One flag,
  `PersistenceMode`, gates every save. Rationale: real documents from day one without risking data
  the Tauri app is autosaving concurrently.
- **Minimal backend scope first.** Only config, document storage and the new document model were
  brought over; lsp/mcp/acp/db/ssh/import/multiplayer come with the node that needs them.
- **Nodes are real elements scaled through rem size**, not custom-painted. Keeps native inputs,
  tables, scrolling and hit-testing; the trade-off is hairline borders that do not scale (a
  feature).
- **Under `--performance`, nodes stop drawing their bodies below zoom 0.32**, which reverses an earlier decision here
  that the placeholder LOD was "a limit of the browser renderer, not of gpui". That was an
  assumption, and it was wrong: the cost is in building and laying out the element trees, which
  no renderer avoids, and culling works against you when zooming out because more cards fit the
  screen the further back you go. Measured on a page of 24 result nodes, a pinch from 100 % to
  10 % cost 39.5 ms a frame; removing the per-frame waste took it to 30.8, and not building
  unreadable bodies took it to 14.3. It is behind a flag because the trade is visible: a canvas
  of shells is harder to read at a glance than a canvas of nodes, and that is the user's call.
  See `peek-canvas/src/lod.rs` and `docs/canvas.md`.
- **Strict lints, verbatim from the Tauri workspace.** Item-level allows with reasons only.
  Clippy's `wildcard_imports` means `use gpui_kit::prelude::*;` plus explicit type imports
  instead of the docs' `use gpui_kit::*;`.

## Design choices worth knowing

- **Actions vs mutation API** are deliberately two layers (see commands.md). It keeps MCP,
  multiplayer and undo off the focus/dispatch path.
- **Action names equal keymap ids.** `actions!(Zoom, [FitView])` yields `"Zoom::FitView"`, so
  the existing `settings.json` keymap works without a translation table for ids; only key
  strings are translated (`meta`→`cmd`, `arrowleft`→`left`).
- **Canonical modifier order is `ctrl-alt-cmd-shift`** (Zed's spelling). gpui parses any order;
  the fixed order exists so user overrides merge by key correctly.
- **Wheel delta is not negated.** gpui already delivers the direction content should move on
  macOS natural scrolling; the first build inverted it and the user reported it.
- **Viewport commits only on gesture end.** Matches React Flow's `onMoveEnd` and keeps the
  document revision quiet during drags.
- **`f64` geometry in the document model.** JavaScript doubles must round-trip exactly; `f32` broke
  the clean-write test. Conversion to gpui `f32` pixels happens in one file.
- **`serde_json` `preserve_order` is enabled explicitly**, in the workspace manifest, not left to
  gpui to pull in transitively. It was transitive once, and that is not the same thing: the
  feature reached `peek-ui`'s build graph but not `cargo test -p peek-document`, so the same code
  sorted a chart's columns in one build and preserved them in the other. Column order is
  load-bearing — `BarChartNode.tsx` reads its axis and primary series from it — so writing a
  document back with sorted keys silently re-labels a chart the TypeScript app then reopens.
  `chart_rows_keep_the_querys_column_order` fails loudly if the feature ever goes away.
- **Node regions are hit-tested in world space, not by node listeners.** `CanvasElement` registers
  its window-level mouse listeners at the end of `paint`, and gpui dispatches the bubble phase in
  reverse registration order, so a node's own `on_mouse_down` can never run before the canvas's.
  Resolving header / body / resize from the pointer's world position sidesteps the ordering
  entirely and keeps the rule unit-testable without a window.
- **Undo is snapshots, not a command log.** `Document::update_data` takes a closure, which has no
  inverse, and a snapshot restores correctly no matter who wrote the change — which is what
  multiplayer will need. It is per page and holds `{nodes, edges, regions}` only, matching
  `useUndoHistory.ts`: no viewport (so panning is never undoable) and no selection. Cost is about
  2 MB for 50 snapshots of the 27-node fixture page and about 17 MB for the agent-heavy page in
  `plock/production.json`; if that ever matters the fix is `Arc<Node>` inside the snapshot, local
  to `history.rs`.
- **A layout-derived size is not a user edit.** `Document::set_intrinsic_size` persists a size a
  node derived from its own content — the Text node widening to fit the line being typed — and
  opens no undo transaction. Recording it would cut a real edit in half: `EditKind::Resize`
  differs from the `EditKind::Data` of the typing that caused it, so it would seal that
  transaction from about the fifth character onwards, and every text node grows while you type
  into it. Opening no transaction folds the grow into whichever one is already open, so undoing
  the typing also restores the width. That matches `useUndoHistory.ts`, which has no edit-kind
  concept at all and simply debounces the page snapshot — the splitting was introduced by this
  port, not inherited from it.
- **`Instant::now()` is called in exactly one place**, `Document::begin`, and passed into
  `History::record`. That makes the 300 ms coalescing deterministic in tests with synthetic
  instants and no sleeps.
- **Nodes are placed on the drag's first frame, not on mouse-up**, as the TypeScript app does:
  a rectangle drawn through the marquee overlay says nothing about what will appear, and a node
  that is real from the first frame shows its kind, its header and its minimum size while it is
  being sized. It costs the reducer an `Effect` triple (`PlaceNode`, `ResizePlacement`,
  `CommitPlacement`) and the view the `NodeId` the document minted; it costs undo nothing,
  because `resize_placement` opens no transaction and the creation's `Structure` one is sealed
  by the release.
- **Writing is a CLI flag, not a settings key.** `settings.json` is shared with the Tauri app,
  which deserialises into a typed struct and would drop an unknown key on its next save. One
  mechanism, not two: `--write` opts in, and `WorkspaceView::with_document` — the constructor
  tests use — stays read-only, so no headless test can touch `~/peek`.
- **The palette dispatches through the canvas focus handle**, not the dialog's focus, because
  dialogs are siblings of the app view under `Root`. `Root` also does not mount its own dialog
  layer; `WorkspaceView` renders `Root::render_dialog_layer` and friends.
- **Theme is two globals** because gpui-component widgets only read its `Theme`, while the canvas
  needs roles that theme has no slot for (grid, node surfaces, kind accents, regions, brackets).
- **Themes are Rust consts, not JSON**, so a missing colour is a compile error; a future user-theme
  loader can deserialize into the same `ThemeSpec`.
- **Contrast test floor is WCAG AA** because Paper's ink-on-cream ships at 6.1:1 in the original.
- **No system-wide synthetic input for verification.** An early attempt sent keystrokes to the
  wrong app. Headless `#[gpui_kit::test]` is the verification path.

## Verified gpui/gpui-kit facts the design relies on

- gpui-pre 0.3.4 has `Window::with_rem_size`, `AnyElement::layout_as_root` / `prepaint_at`,
  `on_pinch`/`PinchEvent { position, delta, modifiers, phase }`, `request_animation_frame`
  (notifies the current view), `App::reduce_motion()`, `window.on_mouse_event` (paint phase),
  `HitboxBehavior::Normal`/`BlockMouse` (node shells started on
  `Div::block_mouse_except_scroll` and no longer use it: it stopped the canvas hitbox from being
  hovered over a node, so presses never reached the view).
- `KeyBinding::new` takes a concrete action type; boxed actions go through `KeyBinding::load`
  with `cx.keyboard_mapper()`.
- gpui-component: `Command` palette (`CommandItem`, `on_select`/`on_confirm`/`on_cancel`,
  `CommandState::set_selected_index(Some(IndexPath::new(row)))`), `Dialog` via
  `window.open_dialog(cx, |dialog, ..| ..)` with `.content(|content, ..| content.child(..))`,
  `TitleBar::window_options()`, `Theme::apply_config` + `Theme::change`, `ThemeConfigColors`
  deserialises from dotted keys and keeps `base.*` private.
- Vendored sources for checking signatures live under
  `~/.cargo/registry/src/*/{gpui-pre-0.3.4,gpui-component-0.6.1,gpui-kit-0.6.1,gpui-base-0.6.1}`.
