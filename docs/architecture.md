# Architecture

## Crate map

Split by compile time: the crate edited most (UI) is a leaf, and everything testable without a
window lives below gpui. Only `peek-ui`, `peek-theme` and the binary depend on gpui-kit.

```
peek (bin)   src/main.rs — parses --workspace/--connection, calls peek_ui::run
└── peek-ui          gpui views: WorkspaceView, CanvasView + CanvasElement, NodeShell, HUD,
    │                theme picker, commands registry, key binding.            [gpui-kit]
    ├── peek-theme   ThemeSpec tables, PeekTheme global, ThemeConfig mapping,  [gpui-kit]
    │                ThemeService (preview/commit)
    ├── peek-canvas  Camera, CameraFlight, gesture reducer, hit tests, grid spacing,
    │                session Document (selection + revision), Scope           [no gpui]
    │   └── peek-document  CanvasDocument serde model, ids, geometry,
    │                      DocumentStore (paths, legacy-dir migration)         [no gpui]
    │       └── peek-config  settings.json model, ThemeId, PersistenceMode,
    │                        Peek→gpui keystroke translation                   [no gpui]
    ├── peek-lsp    tree-sitter SQL: completions, diagnostics, formatting,
    │               the highlight query and the destructive-write guard      [no gpui]
    ├── peek-db     sqlx Postgres/MySQL drivers, SSH tunnels, schema introspection,
    │               the UPDATE/DELETE builders, and the tokio runtime every
    │               database call runs on. Depends on peek-document for the
    │               result model, so peek-canvas reads rows without sqlx.    [no gpui]
    └── (later) peek-mcp, peek-acp, peek-multiplayer — backends, never gpui
```

Workspace lints are the Tauri app's, verbatim and deny-by-default (clippy all + pedantic,
`dead_code`, `unreachable_pub`, `missing_debug_implementations`, `unsafe_code`, …). Item-level
allows only, always with `reason = "..."`. See `CLAUDE.md` for the escape-hatch list.

Dev profile pins gpui's hot crates at `opt-level = 3` (gpui-kit's own recommendation) and leaves
workspace crates at 0; a cold gpui-kit build is about a minute, incremental UI builds a few seconds.

## Data flow

```
~/peek/settings.json ──peek-config──▶ PeekConfig { theme, keymap overrides, workspaces, … }
~/peek/workspaces/<ws>/<conn>.json ──peek-document──▶ CanvasDocument ──normalize──▶
    peek_canvas::Document (adds session selection + revision) held as Entity<Document>
        ▲                                                       │
        │ mutation API (create_node, remove_nodes,              │ read in CanvasView::render
        │ update_data::<D>, connect, set_bounds, undo, …)       ▼
   CanvasView ◀── Effects ── gesture::reduce ◀── Input ◀── window-level mouse listeners
        │                                                  (registered by CanvasElement::paint)
        ├── camera: Camera / CameraFlight (world ↔ screen, flights ticked in render)
        └── renders CanvasElement { items: NodeShell AnyElements, overlay }
```

Two layers are deliberately separate:

- **Actions** (user intent) are gpui `Action` types dispatched through focus paths and handled by
  views. Keyboard, palette, menus and buttons all dispatch the same action.
- **Mutation API** (`peek_canvas::Document` methods) is synchronous and window-free. Action
  handlers are thin: resolve the target, call the mutation, `cx.notify()`. Later, MCP tools,
  multiplayer and undo call the mutation API directly and never synthesize actions.

## Key types and where they live

| Type | Crate / file | Notes |
|---|---|---|
| `PeekConfig`, `PersistenceMode`, `ThemeId`, `gpui_keystroke` | `peek-config/src/{lib,persistence,theme_id,keymap}.rs` | Ported from `src-tauri/src/config/mod.rs`; serde shape frozen |
| `CanvasDocument`, `Page`, `Node`, `NodeKind`, `Edge`, `Region`, `Viewport` | `peek-document/src/{document,node,edge,region}.rs` | Frozen on-disk shape, see below |
| `NodeType` + default/min sizes, `FALLBACK_SIZE` | `peek-document/src/kinds.rs` | From `defaults.ts`, `nodeGeometry.ts` |
| `Point`, `Size`, `Rect` (f64) | `peek-document/src/geometry.rs` | Converted to gpui pixels only in `peek-ui/src/canvas/convert.rs` |
| `DocumentStore`, `DocumentFile` | `peek-document/src/storage.rs` | Paths, `workspaces/` migration, read-only gate, atomic write, mtime guard, one-time backup |
| `NodeData` | `peek-document/src/node.rs` | Names a kind's payload by type, so `Document::update_data::<D>` stays closed over the eleven kinds |
| `History`, `EditKind` | `peek-canvas/src/history.rs` | Per-page snapshots, 50 deep, coalesced over 300 ms |
| `Hit`, `NodeHit`, `NodeRegion`, `Corner` | `peek-canvas/src/hit.rs` | World-space header / body / resize bands, then the edge layer beneath them |
| `EdgeCurve`, `curve_between` | `peek-canvas/src/edge.rs` | The floating bezier, shared by painting and hit-testing |
| `Autosave` | `peek-ui/src/autosave.rs` | 3 s debounce off the revision counter, flushed on quit |
| `Camera`, `FitOptions` | `peek-canvas/src/camera.rs` | React Flow semantics: `screen = world * zoom + pan` |
| `CameraFlight`, `Easing`, `durations` | `peek-canvas/src/flight.rs` | Centre lerp + geometric zoom |
| `Interaction`, `Input`, `Effect`, `reduce` | `peek-canvas/src/gesture.rs` | Pure pointer/wheel state machine |
| `Document`, `Scope` | `peek-canvas/src/{model,scope}.rs` | Session state and command availability counters |
| `Command`, `COMMANDS`, contexts | `peek-ui/src/commands/mod.rs` | The registry |
| `actions::*` | `peek-ui/src/commands/actions.rs` | `actions!(Group, [Variant])` per Peek group |
| `WorkspaceView` | `peek-ui/src/workspace.rs` | Title bar, canvas, palette, theme picker, overlay layers |
| `CanvasView`, `CanvasElement` | `peek-ui/src/canvas/{mod,element}.rs` | See canvas.md |
| `NodeShell` | `peek-ui/src/node/mod.rs` | Header + body slot, brackets, tick/dot; kinds fan out in `node/kind.rs`, one folder each |
| `ThemeSpec`, `PeekTheme`, `ThemeService` | `peek-theme/src/{spec,resolved,service}.rs` | See themes.md |
| `Backend`, `SchemaIndex`, `SharedSchema` | `peek-lsp/src/{backend,schema}.rs` | One per process, held as a gpui global; M5 fills the schema |
| `QueryState`, `SqlCompletions` | `peek-ui/src/node/query/{mod,language}.rs` | The SQL editor and its bridge to peek-lsp |
| `ResultState`, `ResultDelegate` | `peek-ui/src/node/result/` | The virtualised results table over `Document::result` |
| `Database`, `Session` | `peek-ui/src/database.rs`, `peek-db/src/session.rs` | One connection for the process; the tokio runtime lives in peek-db |
| `Plan`, `run` | `peek-ui/src/execution.rs` | Variable resolution, the unbounded-write gate, and the round trip |
| `place_result`, `result_size` | `peek-canvas/src/execution.rs` | Where a query's output lands and how big it is |

## The frozen document format

Source of truth: `~/labs/peek/src/canvas/types.ts`. Both apps must read and write the same files.

```
CanvasDocument { version: 1, activePageId, pageOrder: [PageId], pages: { PageId: Page } }
Page { id, name, nodes: [Node], edges: [Edge], viewport: { x, y, zoom }, regions?: [Region] }
Node { id, type, position: {x, y}, width?, height?, data: <per kind>,
       measured?, selected?, dragging?, resizing?, className?, style? }   ← React Flow extras
Edge { id: "<source>-><target>", source, target, type?, selected? }
Region { id, name, desc, colorIndex, status: "confirmed"|"suggested", memberIds }
```

Rules the Rust model enforces (`peek-document/tests/roundtrip.rs` proves them against a real
workspace file):

- `measured`, `selected` are read (size fallback, session selection seed) and never written;
  `dragging`, `resizing`, `className`, `style` are ignored and dropped on write.
- Size resolution is `measured ?? width/height ?? 200` per axis.
- `NodeKind` is adjacently tagged (`type`/`data`) and flattened into `Node`; unknown kinds parse
  as `NodeKind::Unknown` and are dropped by `normalize`, which also clears stale `isRunning`.
- `liveIntervalMs` distinguishes absent from `null` (`LiveInterval::Off`).
- Numbers are `f64` so JavaScript doubles round-trip byte-for-byte.
- Result rows are a sidecar `<conn>.results.json` (1–13 MB), read and written by
  `ResultSidecar`/`ResultsFile` and held on the session `Document` under their own revision
  counter, never in a history snapshot.
- Ids are `<prefix>_<nanoid(8)>` with the JS alphabet; result nodes are `<queryId>-result-<n>`.
