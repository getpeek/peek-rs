# Porting the Tauri host

Everything below refers to `~/labs/peek/src-tauri/`. Verdicts were verified by reading the code.

| Module | Lines | Tauri coupling | Verdict | Status in peek-rs |
|---|---|---|---|---|
| `crates/lsp` (peek-lsp: tree-sitter SQL, completions, diagnostics) | 2,411 | none | copy verbatim | **done** → `peek-lsp` (see below) |
| `crates/mcp` (peek-mcp: tower-mcp server, 21 tools, `FrontendBridge` trait) | ~970 | none | copy verbatim; implement `FrontendBridge` over the `Document` mutation API | not yet (M6) |
| `crates/acp` (peek-acp: ACP client, `AcpHost` trait) | ~424 | none | copy verbatim | not yet (M6) |
| `src/database/` (`trait Database`, Postgres + MySQL via sqlx) | 686 | none | copied; **columnar row format adopted**, typed errors, decode-failure distinguished from NULL, arms added for arrays/`bytea`/`interval`/enums | **done** → `peek-db` |
| `src/ssh_tunnel.rs` (russh) | 235 | none | copied; **host-key verification added** (`HostKeyPolicy`, trust-on-first-use, always refuses a *changed* key) and a graceful `close` so the fixed `local_port` is free before the next tunnel binds it | **done** → `peek-db::tunnel` |
| `src/import/` (csv/json → temp table) | 193 | none | copy | not yet (M5) |
| `src/config/mod.rs` | 508 | `#[tauri::command]` ×10, `AppHandle` on `set_theme` | strip glue | **done** → `peek-config` |
| `src/config/keymap.rs` (typed `Action` enum + defaults) | 268 | none | not ported: gpui `actions!` and the registry own ids and defaults | replaced |
| `src/storage_commands.rs` | 230 | wrappers | port paths + migration; document schema had to move into Rust | **done** → `peek-document` (history log paths not yet) |
| `src/database_commands.rs` | 89 | wrappers | became `peek_db::Session` methods; the runtime lives there so UI code needs no tokio | **done** (execution not yet wired to the canvas) |
| `src/lsp_commands.rs` | 78 | wrappers | not ported: the UI calls `Backend` directly | replaced |
| `src/mcp_commands.rs` | 70 | `emit("mcp:request")` bridge | rewrite as in-process `FrontendBridge` | not yet |
| `src/acp_commands.rs` | 425 | `emit` host + `State` | rewrite the ~95-line host; **keep the ~180-line login-shell `PATH` recovery** (Dock-launched apps have a stripped `PATH`) | not yet |
| `src/multiplayer/` (iroh docs + gossip) | 766 | `AppHandle` threaded through every loop, 8 `emit` sites | replace with an event-sink trait or channel; drop the base64 hop | not yet (M7) |
| `src/dock_icon.rs` | 48 | `AppHandle::run_on_main_thread` | port with objc2 under an item-level `unsafe` allow | not yet |
| `src/window_chrome.rs` | 60 | WKWebView layer hacks | delete; gpui owns the window | n/a |
| `src/lib.rs` / `main.rs` | 242 | everything | rewritten | **done** (`peek-ui::run`) |

Porting policy: copy the files, delete `#[tauri::command]` functions and `AppHandle`/`State`
parameters (the pure functions beneath them stay), take `base: &Path` instead of reading `HOME`
inside functions so they are testable, and keep user-data migrations (the `workspaces/` move) —
those are not compat shims.

## Frontend behaviour that still has to be reproduced

The React app is the behaviour spec. Most useful files:

| Feature | Read |
|---|---|
| 30-method canvas API (the natural trait boundary) | `src/canvas/state.ts:252-292`, `src/canvas/hooks/useCanvas.ts` |
| Edges: floating bezier from rect intersections, tinted by **target** kind; only variable→query and result→chart carry data | `src/canvas/edges/FloatingEdge.tsx`, `src/canvas/variables.ts`, `src/canvas/hooks/useChartSync.ts` |
| Regions & wayfinding (padding 56, cross-fade 0.35→0.21, beacons, edge peekers) | `src/canvas/wayfinding/` |
| Undo (50 snapshots, 300 ms coalescing) and version history (JSONL, full every 20) | `src/canvas/ui/useUndoHistory.ts`, `src/canvas/history/` |
| Autosave (3 s debounce, document + results sidecar) | `src/canvas/hooks/useAutoSaveDocument.ts` |
| Query execution and result placement (`x + width + 50`, per-type column widths) — **ported**, `peek_canvas::execution` + `peek_ui::execution` | `src/canvas/executeQueries.ts` |
| Jump labels (`g`), page search (`cmd-f`), minimap (176×116) | `src/canvas/jump/`, `src/page-search/`, `src/canvas/minimap/` |
| Node kinds and their `data` | `src/canvas/types.ts`, `src/canvas/nodes/<Kind>/` |

## peek-lsp, as ported

Copied module for module. Four deltas, all forced or additive:

- **`tree-sitter` 0.25 → 0.26.** Not a choice: the crate sets `links = "tree-sitter"`, so one
  version must serve the whole graph, and gpui-component pins 0.26.13. The upside is that
  `peek-lsp` and gpui-component's highlighter now parse SQL with the same
  `tree-sitter-sequel` grammar object. The 86 copied tests pass unchanged on it.
- **No `unsafe`, and no allow for it.** `tree_sitter_sequel::LANGUAGE.into()` is safe through
  `tree-sitter-language`. The workspace `Cargo.toml` comment claiming this crate needed an
  audited `unsafe` block was inherited from the Tauri app and was wrong in both repos; only the
  dock-icon module justifies `deny` over `forbid`.
- **`Backend::completion_at_offset`** beside `completion`. gpui-base hands a completion provider
  a byte offset and every step below wants one too, so going through the `Position` form would
  convert to UTF-16 and straight back.
- **`sql_text` and `highlights`**, new modules: `format` (the `sql-formatter` behaviour, via the
  `sqlformat` crate, with `formatPreservingVars`' `@variable` placeholder swap),
  `is_unbounded_write` (ported from `isUnboundedWrite.ts`, ready for M5's confirmation gate),
  `variable_sites`, and `sql_highlights`.

`SchemaIndex` is reached through `SharedSchema`/`shared_schema`/`set_schema` so callers never
name `parking_lot`. M5 fills it; nothing else changes.

Not ported and not needed: `lspBridge.ts`'s LSP→Monaco kind mapping (gpui-base speaks
`lsp-types` 0.97, the same version `peek-lsp` does, so completions and diagnostics pass through
unconverted), `overflowWidgets.ts`, and most of `editor.css`.
