# Commands, actions and the keymap

## Principle

One logical command = one gpui `Action` = one entry in `crates/peek-ui/src/commands/registry/`.
Keyboard, palette, menus and buttons all dispatch that action; availability, title, group and
default keys come from the one registry entry.

## Actions

`crates/peek-ui/src/commands/actions.rs` declares one module per Peek group with
`actions!(Group, [Variant, …])`. gpui names such an action `"Group::Variant"`, which is
byte-identical to the ids Peek already uses in `settings.json`'s `keymap` and in
`docs/keymap.md` of the Tauri app. `Edit::Copy` lives in a nested `edit::copy` module because `Copy` shadows the marker
trait inside the module that declares it; its id is still exactly `"Edit::Copy"`, which is what
`settings.json` binds. Every action the build knows is declared here at once, including ones
whose entry and handler land later — one file is the single place a name is coined.

## Registry

Entries live one file per group under `commands/registry/`, collected by that module's
`GROUPS`; `commands::all()` walks them and is what the palette, the keymap, the toolbar and the
registry tests read. **Add a command by appending to its group's `ENTRIES` — never by rewriting
the file.** A single array was a merge conflict every time two features landed at once, and
reconstructing one from memory silently drops whatever else was in it.

```rust
pub struct Command {
    pub id: &'static str,             // "Zoom::FitView"
    pub title: &'static str,          // the stable name: tooltips, the keymap modal
    pub label: Option<fn(&Scope) -> &'static str>, // palette label when it follows state
    pub group: Group,
    pub keywords: &'static str,       // extra palette search terms, whitespace-separated
    pub default_keys: &'static [&'static str], // Peek syntax ("meta-shift-0")
    pub context: &'static str,        // gpui key-context predicate
    pub build: fn() -> Box<dyn Action>,
    pub available: fn(&Scope) -> bool,
}
```

`label` is what makes a toggle name what pressing it *does* rather than what it controls —
"Hide UI" against "Show UI". It takes a `Scope` and no `&App`, deliberately: that is what keeps
the registry free of gpui. **The price is that any setting a label reads has to be projected
into `Scope` first**, which is what `Scope::settings` (`SettingsScope`) exists for;
`CanvasView::scope` fills it from the `Settings` global. Adding a stateful label to a new
preference means adding a field there, not widening the signature.

`Scope` (`peek-canvas/src/scope.rs`) is a handful of counters (`selected`, `selected_queries`,
`pages`, `camera_locked`, …) computed by `Document::scope()`; the palette filters on it at open
time and buttons may read it in render. Tests assert every `(build)().name() == id` and that every
default key translates.

## Palette search

`Scope` decides what the palette *offers*; what it *shows* is decided by `commands/palette.rs`,
which ranks with `crate::fuzzy` — the same subsequence scorer behind the connection picker, the
result find bar and page search. gpui-component's own filter is a case-insensitive `contains` over
the label, which neither finds "Fit all nodes in view" from `fitv` nor puts the best answer first,
so `Command::filterable(false)` switches it off and `on_query` installs our ranking instead.

Each keyword is scored as its own haystack, not as one joined string: a subsequence is free to
wander across two unrelated terms otherwise, and a short keyword's density would be lost. Keyword
hits then take a small discount (`KEYWORD_WEIGHT`), because keywords are short enough to score
near-perfectly and would otherwise bury the row whose visible title the reader was typing. A row
matched only by a keyword draws unhighlighted — there is nothing on it holding those characters.

The dialog is mounted inside `WorkspaceView::render`, so re-ranking has to end in a repaint of the
workspace; `CommandState` re-rendering itself is not enough to rebuild the row list. The ranked
rows live in their own `Listing` entity for the same reason — the dialog's content builder runs
while the workspace is rendering, and reading the entity being rendered is a double lease.

Implemented ids: `Zoom::{In,Out,Reset,FitView,FitSelection,FitSelectionAndLock}`,
`Edit::{SelectAll,DeleteSelection,Copy}`, `History::{Undo,Redo}`,
`Tool::Select` (escape → clear selection), `Tool::{Query,Agent,Text,Variable,Draw}`,
`Page::{New,Close,Previous,Next,OpenPicker,GoToNode,Search}`,
`Page::{SelectNodeLeft,SelectNodeRight,SelectNodeUp,SelectNodeDown}`,
`Page::{SelectPreviousQuery,SelectNextQuery}`,
`Query::{Focus,Format,Run,RerunAll,RerunSelected}`, `Result::Pivot`,
`Export::{Csv,Json}`, `View::{ToggleCameraLock,ToggleUi,Organize,Schema}`,
`Settings::{TogglePageDisplay,ToggleCommandPaletteButton}`, `Help::Keymap`,
`Agent::{Fork,CycleMode,Stop}`, `CommandPalette::Open`, `Theme::Open`,
`ConnectionPicker::Open`, `App::{About,Quit}`.

`Page::GoTo` is the one **data-carrying** action (`GoTo { page: PageId }`, `no_json`). It has no
registry entry and no default key: the palette generates one row per page, and there is nothing
stable for a user to bind.

Not implemented, each blocked on a feature rather than on the command:
`Tool::LassoSelect` (no selection-tool state, no freehand `Interaction`, nothing to paint the
path), `Region::{GroupSelection,UngroupSelection,OpenPicker}` (regions have a mutation API but
no renderer), `View::ShowRunningQueries` (the Activity node is still a placeholder), the minimap
and history-panel toggles, and the collaboration commands. Registering any of them now would put
a row in the palette that does nothing, which is the failure the `Command`/handler pairing
exists to prevent.

`ConnectionPicker::Open` is the one command on `WORKSPACE_NOT_TYPING`: bare `p`, handled on
`WorkspaceView` because switching a connection rebuilds the document, its pages, the rows sidecar
and the autosave — none of which the canvas can reach. A `CANVAS`-scoped binding would only fire
while the canvas subtree held focus, and the picker has to open from the title bar too. The
`!Input` clause is also what lets a `p` typed into the picker's own search box insert a letter
instead of toggling the panel shut.

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

## Where a handler lives

Two facts about gpui's action dispatch decide this, and both cost real rework to learn.

**Dispatch runs from the focused element outward through its ancestors, never inward.** The
palette confirms through `WorkspaceView::canvas_focus`, so:

- a handler on `WorkspaceView` **is** reachable from the palette — the canvas is a descendant of
  it. `CommandPalette::Open`, `Theme::Open` and `ConnectionPicker::Open` all work this way.
- a handler on a **node view is not** — nodes are children of the canvas. An action handled only
  there is listed by the palette and then silently does nothing.

So `canvas/dispatch/` holds the handlers for commands that act on the *selection or the
document*, because `CanvasView` owns the document entity, `node_states` and the camera — not
because it is the only reachable host. A node keeps its own handler where the command must also
fire *while its editor has focus*; `Query::Run` and `Query::Format` have both, the node's
winning on depth.

**Actions stop propagating by default.** This is the opposite of mouse and key events, and it is
easy to get backwards:

```rust
// gpui-pre-0.3.4/src/window.rs:6138
cx.propagate_event = false; // Actions stop propagation by default during the bubble phase
```

Listeners run leaf→root and the first one ends the dispatch unless it calls `cx.propagate()`.
A deeper handler therefore wins **without** `cx.stop_propagation()`, and adding one *to an
`on_action`* is a no-op that reads as load-bearing — the next person copies it and treats its
absence elsewhere as a bug. Pin the precedence with a test instead.

**The trap next to the trap: the rule is per listener kind, not per file.** Actions stop by
default; mouse and key events do not. A `stop_propagation` in an `on_click` is usually the only
thing keeping a nested control from also firing its parent — the connection picker's pencil and
duplicate buttons sit inside a row whose own click switches connections, and without it, editing
a connection would switch to it as a side effect (`opening_the_form_does_not_switch_to_the_connection`
pins that). So: delete one from an `on_action`, keep one in an `on_click`, and never move a call
between the two without re-reading which kind of listener it now sits in. `Page::Search` is the worked example: one registry
entry on `CANVAS_NOT_TYPING`, a page-wide handler on the canvas, and a result-node handler that
wins when the table has focus — except in the record view, where it calls `cx.propagate()` to
hand the key on. The pair of tests in `node/result/mod.rs` is what makes that invariant legible,
and the `cx.propagate()` call is the only difference between them.

This also closes a dead zone in the reference, which arbitrates the same key with two
independent listeners whose guards (`selected && !pivoted`, and `nothing selected`) are not
complementary: with a pivoted result selected, both decline and `⌘F` does nothing. Here the
discriminator is focus, so exactly one handler always runs.

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

- **Palette** (`WorkspaceView::open_palette`): snapshots `Scope`, then asks
  `commands::palette::entries(document, &scope)` for a `Vec<PaletteEntry>` — registry commands
  that pass `available`, each carrying its `label(scope)`, followed by whatever the `DYNAMIC`
  providers append. A `PaletteEntry` is `{ title, keywords, action: Box<dyn Action> }`, which is
  what lets a runtime-generated row (one "Go to <page>" per page) sit in the same list as a
  static one and dispatch the same way. Registering a provider is all a feature needs to do.
  **Generate rows sparingly**: the reference's palette returns nothing for an empty query and is
  search-only, so a row per connection or per node would swamp a surface nobody browses — "Change
  connection" is deliberately one row that opens the picker. It then builds `CommandItem`s and
  opens a gpui-component `Command` inside a `Dialog`. Confirm closes the
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
