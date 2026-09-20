# About Peek (Rust)

Peek is a Figma-like database GUI: an infinite 2D canvas of typed nodes (query, result,
text, variable, agent, chart, …) backed by a live database connection. This repository is
the from-scratch pure-Rust rewrite of the Tauri + React app at `~/labs/peek`, built on
**gpui-kit 0.6.1** (gpui-pre 0.3.4 + gpui-component 0.6.1). macOS 15+, Rust 1.90+, edition 2024.

Project documentation for humans and agents lives in `docs/` (start with `docs/README.md`:
status, architecture, canvas, commands, themes, testing, porting, decisions). Keep `docs/status.md`
current when a milestone or a known gap changes.

# The behaviour reference: `~/labs/peek`

`~/labs/peek` is the specification and is **read-only**. When porting a feature, read the
TypeScript source first and reproduce its behaviour, constants and on-disk formats:

| Concern                                 | Where to look                                                                         |
| --------------------------------------- | ------------------------------------------------------------------------------------- |
| Canvas, document model, camera verbs    | `~/labs/peek/src/canvas/` (`types.ts`, `defaults.ts`, `ids.ts`, `hooks/useCanvas.ts`) |
| Node kinds                              | `~/labs/peek/src/canvas/nodes/<Kind>/`                                                |
| Command palette commands                | `~/labs/peek/src/command-palette/commands/`                                           |
| Keyboard shortcuts and keymap semantics | `~/labs/peek/docs/keymap.md`, `src-tauri/src/config/keymap.rs`                        |
| Themes (`--pk-*` tokens)                | `~/labs/peek/src/canvas/nodes/theme/*.css`, `node.css`                                |
| Rust host already ported / to port      | `~/labs/peek/src-tauri/src/`, `src-tauri/crates/{lsp,mcp,acp}`                        |
| User data                               | `~/peek/settings.json`, `~/peek/workspaces/<workspace>/<connection>.json`             |

**On-disk formats are frozen.** `settings.json` and workspace documents must stay readable by
both apps. Never write a document shape the TypeScript app cannot open. Ephemeral React Flow
fields (`selected`, `dragging`, `resizing`, `measured`, `className`, `style`) are tolerated on
read and dropped on write.

Until the milestone that enables writes, the app runs with `PersistenceMode::ReadOnly`: it
reads the real `~/peek` but never writes to it. Every save path is gated on that flag.

Backend modules from the Tauri app are Tauri-free and are copied verbatim when a milestone needs
them (`crates/lsp`, `crates/mcp`, `crates/acp`, `database/`, `ssh_tunnel.rs`, `import/`).
`config/` and `storage_commands.rs` were ported by stripping `#[tauri::command]`/`AppHandle`.
`multiplayer/` needs its `AppHandle` replaced by an event-sink trait. `window_chrome.rs` and
`lib.rs` are not ported.

# Milestones

- [x] M1 Load a real document; placeholder node shells; pan / zoom / camera flights; keymap + palette (headless tests green; manual feel-test pending)
- [x] M2 Node shell: header, selection (click / shift / marquee), drag, entry animation
- [x] M3 Themes: the six Peek themes as `PeekTheme` + gpui-component `ThemeConfig`, picker with live preview (syntax colours, swatches and dock icon trail)
- [x] M4 First real nodes (Text, Variable, Query + peek-lsp), document mutations, undo, autosave → writes enabled
      (Query runs nothing until M5: no connection, so `Query::Run` does not exist yet)
- [x] M5 peek-db: connections, SSH tunnels, results, schema, import
      (done: drivers, tunnels + host-key verification, schema, the results sidecar and its autosave,
      the mutation SQL builders, the tokio↔gpui session bridge, `Query::Run` with result placement,
      live polling and the unbounded-write gate, and the Result node's table with rectangular cell
      and row selection, TSV copy, persisted column widths, the toolbar with selection statistics,
      in-result find, PK/FK classification, the cell value pane, inline editing and row deletion.
      Left: import, and the Result node's export / context menus / pivot — see docs/status.md)
- [x] M6 peek-mcp bridge, peek-acp agent node, local Ollama backend
      (the 21 canvas tools live in `peek-canvas::tools` and serve both the MCP bridge and the
      agent node's own loop)
- [x] Regions and wayfinding: derived boxes, halos, beacons, edge peekers, the picker in the zoom
      cluster, the Keep/Rename/Dismiss card over a suggestion, and
      `Region::{GroupSelection,UngroupSelection,OpenPicker}` + `Settings::ToggleRegions`
- [x] Local AI: the two Ollama groupings (`Region::{GroupWithAi,RegroupAllWithAi}`, prompts and
      geometric fallback in `peek-canvas::regions::grouping`) and automatic query labels
      (`Settings::ToggleAutomaticallyLabelQueries`). Export filenames stay the SQL slug — see
      docs/status.md
- [ ] M7 peek-multiplayer

# Crate map

Split by _compile time_: the crate you edit most (UI) is a leaf, and everything testable without
a window lives below gpui.

```
peek            src/main.rs — a few lines; parses args, calls peek_ui::run
peek-ui         gpui-kit views: workspace, canvas element, node shells, commands registry,
                key binding, theme picker. One of only three crates allowed to depend on gpui-kit.
peek-theme      (M3) ThemeSpec tables, PeekTheme, mapping to gpui-component ThemeConfig. Depends on gpui-kit.
peek-canvas     camera math, flights, gesture reducer, hit testing, LOD, session Document + Scope. No gpui.
peek-document   CanvasDocument serde model, ids, geometry, load/save under ~/peek/workspaces,
                and the result model (`ResultSet`/`Cell`) plus the rows sidecar. No gpui.
peek-config     settings.json model, ThemeId, keymap overrides + Peek→gpui keystroke translation. No gpui.
peek-lsp        tree-sitter SQL: completions, diagnostics, formatting, the highlight query,
                the destructive-write guard. Copied from the Tauri app. No gpui.
peek-db         sqlx drivers, SSH tunnels, schema introspection, the UPDATE/DELETE builders and
                the tokio runtime every database call runs on. Depends on peek-document for
                `ResultSet`. No gpui.
peek-acp / peek-mcp / peek-ollama        agent backends: an ACP subprocess, the MCP server the
                agent drives the canvas through, and a local Ollama client. Each owns its own tokio
                runtime and hands back plain futures. No gpui.
peek-multiplayer backend (later); must never import gpui.
```

Rule: **if it can be written without gpui, it does not go in `peek-ui`.** Camera maths, selection
rules, document mutations and keymap parsing are unit-tested with plain `cargo test` in seconds.

# gpui-kit rules that matter here

- Load the `gpui-kit` skill (`.claude/skills/gpui-kit`) before writing gpui code and the
  `gpui-kit-design-guides` skill before any layout or visual decision. Their
  `references/gpui/*.md` describe the pinned gpui snapshot; prefer them over memory of upstream Zed.
  Vendored source lives under `~/.cargo/registry/src/*/gpui-kit-0.6.1`, `gpui-component-0.6.1`,
  `gpui-pre-0.3.4`. Never invent an API: check the source.
- `gpui_kit::init(cx)` is the first line inside `app.run`; every window's first view is `Root`.
  Import traits with `use gpui_kit::prelude::*;` and name types explicitly (`use gpui_kit::{App, div, px};`);
  clippy's `wildcard_imports` forbids `use gpui_kit::*` outside test modules. Never add `gpui-pre` directly.
- Views that tests must reach register with `.id("…").test_support()` before `.track_focus`; UI behaviour
  is verified with `#[gpui_kit::test]` in `crates/peek-ui/tests/` through real key/pointer dispatch,
  never by sending system-wide keystrokes to the running app.
- Colours only from `cx.theme()` (gpui-component roles) or `cx.peek_theme()` (Peek canvas roles).
  No literal `rgb`/`hsla`/hex in `peek-ui`. Missing role → add it to the theme.
- Sizes use rem helpers (`p_2()`, `text_sm()`, `rems(..)`); pixel constants belong to camera math
  or documented physical boundaries (hairline borders, selection ring width).
- Nodes are real element trees placed at screen coordinates inside `window.with_rem_size(base * zoom)`.
  gpui has no transform groups; do not try to scale elements any other way.
- Stable, domain-derived `ElementId`s (node id, page id). Never list indexes or random ids in `render`.
- Never retain `&mut Window`, `&mut App` or `&mut Context<_>`; hold `Entity`, `WeakEntity`, `FocusHandle`.
- Stateless presentation is `RenderOnce`; anything with retained state is an `Entity<T>` created in `new()`.
- `cx.notify()` only after a real state change, never unconditionally in `render`. Animation frames
  are requested only while a flight or entry animation is active.
- **One logical command = one gpui `Action` = one entry in `crates/peek-ui/src/commands/mod.rs`.**
  Keyboard, palette, menus and buttons all dispatch that action. MCP, multiplayer, undo and tests
  use the `peek_canvas::Document` mutation API directly. Availability comes from `peek_canvas::Scope`.
- Key contexts nest `Workspace > Canvas > <Kind>Node > Input`. Canvas bindings use
  `"Canvas && !Input && !NumberInput"`. `"QueryNode"` is for commands that must fire _while_
  the SQL editor holds focus, so they carry a modifier; `Query::Format` is the only one until
  `Query::Run` joins it in M5.
- Action names generated by `actions!(Group, [Variant])` are `"Group::Variant"` — identical to the
  ids in `settings.json`'s `keymap`. Keep it that way.

# Lints

Workspace lints are the Tauri app's, verbatim and deny-by-default: clippy `all` + `pedantic`,
`dead_code`, `unused_*`, `missing_debug_implementations`, `unreachable_pub`, `unsafe_code`.
Fix the code, not the lint. Allowed escape hatches, item-level only, always with `reason = "..."`:

- `dead_code` for a field wired in a named later milestone; removing the allow is part of that milestone.
- `clippy::cast_*` at module level in `peek-canvas/src/camera.rs` only.
- Hand-written `impl Debug` (`finish_non_exhaustive()`) for types holding closures or `Task`s.
- `#![allow(unsafe_code)]` only in the macOS dock-icon module, with a `// SAFETY:` comment.

Never add a crate-level `#![allow]`. `cargo clippy --workspace --all-targets -- -D warnings` must pass.

# Code style

- Avoid nested `if`s; keep cyclomatic complexity low. Return early.
- Full words for identifiers (`truncated_string`, not `trunc_str`). Spell `Context` out; `cx` is
  only gpui's context parameter.
- **Proximity principle.** Folders and modules are features, not file kinds. No `utils/`, `types/`,
  `helpers/`. A helper with one consumer lives next to it; promote only when a second feature needs it.
- **Composability.** Views stay thin: `render` reads state and composes; logic lives in
  `peek-canvas`/`peek-document` methods or a sibling module.
- Functions take at most three parameters; a fourth means an options struct.
- Comments explain **why**, never what. If deleting the comment would not confuse a reader, delete it.
- `pub` only at crate seams (`unreachable_pub` enforces it). Leaf crates use plain error enums;
  `anyhow` only in the binary and UI. `log` macros, never `println!`.

# Commands

```
cargo run [-- --workspace <name> --connection <name>]      # defaults to the first workspace/connection
cargo test -p peek-config -p peek-document -p peek-canvas -p peek-lsp   # seconds, no gpui
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo build --timings                                      # when compile times regress
```

Always run clippy and fmt after making changes. Run the fast tests when touching the gpui-free crates.

# Don'ts

- Don't add backwards-compat shims, deprecated re-exports or `// removed:` markers. Delete old code.
  The only exception is on-disk user-data migration (the `workspaces/` directory move).
- Don't depend on gpui-kit outside `peek-ui`, `peek-theme` and the binary. Don't add a tokio runtime
  to UI code; gpui has its own executor and backends expose async functions bridged via channels.
- Don't hardcode colours, radii or font stacks. Don't derive `ElementId`s from indexes.
- Don't write speculative abstractions, options no caller passes, or error handling for cases that
  can't happen. Three similar lines beat a premature helper.
- Don't introduce another UI toolkit, state library or async runtime.
- Don't edit `~/labs/peek`. Don't write to `~/peek` while `PersistenceMode::ReadOnly` is in effect.
