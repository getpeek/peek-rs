# Commands, actions and the keymap

## Principle

One logical command = one gpui `Action` = one entry in `crates/peek-ui/src/commands/mod.rs`.
Keyboard, palette, menus and buttons all dispatch that action; availability, title, group and
default keys come from the one registry entry.

## Actions

`crates/peek-ui/src/commands/actions.rs` declares one module per Peek group with
`actions!(Group, [Variant, …])`. gpui names such an action `"Group::Variant"`, which is
byte-identical to the ids Peek already uses in `settings.json`'s `keymap` and in
`docs/keymap.md` of the Tauri app. `Edit::Copy` is `edit::CopySelection` to avoid shadowing the
trait; its registry id can still be `"Edit::Copy"` when it lands.

## Registry

```rust
pub struct Command {
    pub id: &'static str,             // "Zoom::FitView"
    pub title: &'static str,
    pub group: Group,
    pub keywords: &'static str,       // palette search terms
    pub default_keys: &'static [&'static str], // Peek syntax ("meta-shift-0")
    pub context: &'static str,        // gpui key-context predicate
    pub build: fn() -> Box<dyn Action>,
    pub available: fn(&Scope) -> bool,
}
```

`Scope` (`peek-canvas/src/scope.rs`) is a handful of counters (`selected`, `selected_queries`,
`pages`, `camera_locked`, …) computed by `Document::scope()`; the palette filters on it at open
time and buttons may read it in render. Tests assert every `(build)().name() == id` and that every
default key translates.

Implemented ids so far: `Zoom::{In,Out,Reset,FitView,FitSelection}`, `Edit::SelectAll`,
`Edit::DeleteSelection`, `History::{Undo,Redo}`, `Tool::Select` (escape → clear selection),
`Tool::{Query,Agent,Text,Variable,Draw}`, `Page::{New,Close,Previous,Next}`,
`Page::{GoToNode,SelectNodeLeft,SelectNodeRight,SelectNodeUp,SelectNodeDown}`,
`Query::{Focus,Format,Run}`, `Edit::Copy`, `Page::Search`,
`View::{ToggleCameraLock,ToggleUi}`, `Agent::{Fork,CycleMode,Stop}`, `CommandPalette::Open`,
`Theme::Open`, `App::Quit`. The remaining keymap ids from
`src-tauri/src/config/keymap.rs` are added as their features land.

`History::{Undo,Redo}` are bound on `CANVAS`, which excludes `Input`, so
gpui-kit's own `cmd-z` wins while a node editor has focus. `Tool::{Query,Agent,Text,Variable}` arm place
mode rather than creating a node directly; the next click places it. `Tool::Draw` arms the pen
instead, which is sticky — see "The draw tool" in `canvas.md`.

## Key contexts

```
Workspace                  WorkspaceView root       "Workspace"
└─ Canvas                  CanvasView root          "Canvas" (focused by default)
   └─ <Kind>Node           (later) node views
      └─ Input             gpui-kit inputs/editors  "Input"
Dialogs (palette, picker) are siblings under Root, not under Workspace.
```

Registry constants: `WORKSPACE = "Workspace"`, `CANVAS = "Canvas"` (escape only, as a binding),
`CANVAS_NOT_TYPING = "Canvas && !Input && !NumberInput && !JumpMode"`,
`CANVAS_JUMPING = "Canvas JumpMode"`, `QUERY_NODE = "QueryNode"`,
`RESULT_NODE = "ResultNode"`, `AGENT_NODE = "AgentNode"`.

The `_NOT_TYPING` suffix marks the ones that are *predicates* rather than contexts, and the
distinction is load-bearing: `key_context` and the chrome's `tooltip_with_action` both parse
their argument with `KeyContext::parse`, which reads identifiers and `key=value` pairs only.
Given a predicate it reaches the `&`, consumes nothing and recurses on the same input until the
stack is exhausted — the process aborts rather than returning an error. So only a name without
the suffix may leave a `KeyBinding`.

`JumpMode` is not a context any element owns permanently: the canvas swaps its own
`key_context` to `CANVAS_JUMPING` while the jump overlay is up. `KeyContext::parse` splits on
whitespace, so that one div then carries both identifiers, and `Not` scans the whole dispatch
path — which makes every `!JumpMode` binding dead for the duration. That is what lets `q`, `t`,
`v` and `backspace` become label characters without a second focus handle, and it is why the
keys that jump mode handles are read in `on_key_down`: only a key with no live binding ever
reaches a key listener. `escape` is the exception and stays bound (its context is the bare
`Canvas`), so jump mode intercepts it inside the `Tool::Select` handler instead.

`Query::Focus` is a small, deliberate divergence. The reference implements `Enter` as a raw
window listener and documents it as non-configurable; here it is a registry entry like
everything else, so it shows up in the palette and can be rebound. Nothing is written back to
`settings.json`, so the TypeScript app is unaffected.

`QUERY_NODE` is a node-local context: the query node's card carries `key_context("QueryNode")`
and sits above the SQL editor on the focus path, so a command bound there fires *while the
editor holds focus*. That is why `Query::Format` is `meta-s` and `Query::Run` is `meta-enter`
rather than bare keys — a bare key would be swallowed by typing, which is the same reason the
reference gives both a modifier. `Query::Run` is additionally gated on `Scope::connected`, so the
palette does not offer "Run query" on a canvas with no database behind it.
The card is deliberately not focusable itself; the editor owns the focus handle. Single-letter tool keys will use the
`!Input` form so typing is never hijacked; gpui-kit's own `cmd-a`/`cmd-z` bindings on `Input`
sit deeper and win while an editor is focused.

`AGENT_NODE` works the same way, for the same reason: `Agent::CycleMode` (`shift-tab`) and
`Agent::Stop` are pressed *while the composer holds focus*. `shift-tab` is the one binding here
that depends on something subtle — gpui-base binds it to `OutdentInline` in the deeper `Input`
context, and it only reaches the node because `apply_indent` calls `cx.propagate()` for a layout
mode that cannot be indented. `Agent::Fork` is the opposite: it needs the camera, so it is
handled on the canvas, and the node's own header button selects its node before dispatching so
all three surfaces run one path.

`WorkspaceView::new` focuses the canvas; the canvas element is `.id("canvas").test_support()
.track_focus(&focus_handle).key_context("Canvas")`.

## Binding at startup

`commands::keymap::resolved(overrides)` seeds a map from every registry default key (translated
with `peek_config::gpui_keystroke`), then applies the user's `settings.json` overrides by
normalized key, logging and skipping unknown ids or bad combos. `bind` installs them with
`KeyBinding::load` after `gpui_kit::init`, so ties resolve in Peek's favour.
Rebinding on settings change is not implemented (restart applies, as in the Tauri app).

### Keystroke translation

`peek_config::gpui_keystroke("meta-shift-0") == "cmd-)"`. `meta→cmd`, `arrowleft→left`
etc., `esc→escape`, modifiers accepted in any order and emitted as `ctrl-alt-cmd-shift-` so two
spellings of one combo compare equal (this is what makes merge-by-key correct). A doubled trailing
dash means the `-` key; a single trailing dash is an error.

**Shift over a non-letter becomes the character it types, and the shift is dropped**:
`meta-shift-0 → cmd-)`, `meta-shift-[ → cmd-{`. This is not cosmetic — `gpui-pre-macos`'s
`parse_keystroke` only keeps the shift flag when the unmodified key is all ASCII lowercase, so
`cmd-shift-0` is a binding no keypress can ever produce. `Zoom::FitView` and the two page-switch
commands were silently dead until `peek_config::keymap::shifted` landed. UI tests must press what
the platform sends (`window.press("cmd-)")`), since `Keystroke::parse` in the test harness will
happily accept the unmatchable spelling and dispatch it.

## Surfaces

- **Palette** (`WorkspaceView::open_palette`): snapshots `Scope`, builds `CommandItem`s from
  available commands, opens a gpui-component `Command` inside a `Dialog`. Confirm closes the
  dialog first, then dispatches through `canvas_focus.dispatch_action` so the action reaches the
  canvas handlers rather than the dialog's focus. Dialogs only render because `WorkspaceView`
  mounts `Root::render_dialog_layer` (and sheet/notification layers) — `Root` itself does not.
- **Buttons** (HUD, toolbar, page tabs): `focus.dispatch_action(&zoom::In, window, cx)`, always
  through the canvas focus handle, because every canvas action is registered with `.on_action`
  on the canvas element.
- **The toolbar derives itself from this registry.** A tool whose action has a `Command` entry
  is enabled, dispatches it, and shows the key that entry is bound to via
  `Kbd::binding_for_action_in`; one without an entry renders disabled with a tooltip naming what
  it waits for. Registering the command is the only step needed to light a tool up. The
  reference hardcodes its badge letters, so rebinding a key never updates them; resolving the
  live binding avoids inheriting that drift.
- **Menus / context menus** (later): `PopupMenu::action_context(canvas_focus).menu(title, action)`.
- **MCP / multiplayer** (later): call the `Document` mutation API, never actions.
