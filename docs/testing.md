# Testing

Three layers, cheapest first.

## 1. Pure crates (`cargo test -p peek-config -p peek-document -p peek-canvas -p peek-theme`)

Seconds, no window. Camera maths (anchor invariance, clamps, fit), flights, every gesture-table
row, keystroke translation, settings parsing, theme completeness and contrast, and the document
round trip.

The round-trip fixture `crates/peek-document/tests/fixtures/plock-local.json` is a copy of a real
workspace file (SQL text only; a test asserts it contains no connection URLs). The key test:
`clean write == original minus ephemeral fields`, comparing JSON values after number
normalisation, with a recursive diff that names the first differing path.

## 2. Headless UI (`cargo test -p peek-ui --test canvas`)

`#[gpui_kit::test]` with the `test-support` feature (dev-dependency). Each test:

```rust
cx.update(|cx| peek_ui::init(&config, cx));            // gpui-kit, theme, keymap, actions
let handle = cx.open_window(size(px(1200.), px(800.)), |window, cx| {
    let view = cx.new(|cx| WorkspaceView::with_document("test", document, window, cx));
    Root::new(view, window, cx)
});
cx.update_window(handle.into(), |_, window, cx| {
    window.render_frame(cx);
    window.press("cmd-shift-0", cx);                       // real key dispatch through bindings
    window.scroll("canvas", ScrollDelta::Pixels(..), cx);  // needs .id("canvas").test_support()
});
VisualTestContext::from_window(handle.into(), cx).simulate_event(PinchEvent { .. });
```

Covered: fit view flies, commits the viewport and frames all content; wheel pans and camera lock
freezes it; pinch zooms about the pointer and clamps at 4×; `cmd-a`/`escape`; palette open →
escape → `cmd-0` still works; page switching restores each page's viewport; theme picker
previews, reverts and commits.

Flights use wall-clock time, so tests `sleep(350 ms)` then `render_frame` before asserting the
landed camera. `cx.run_until_parked()` after opening or closing dialogs.

When a keybinding silently does nothing, check focus before suspecting the feature. A dead
keystroke reads as a broken feature and usually is not one: during M4, `cmd-z` did nothing while
`Scope::can_undo` stayed `true`, which looks exactly like a corrupted undo stack and was in fact
a node holding a `FocusHandle` that died with it, leaving the window focused on nothing. The
cheap first move is to print `window.focused(cx)` at the moment of the no-op — "the dispatch
never happened" and "the stack is wrong" are very different bugs. See the editable-node
discipline in `canvas.md`.

An undo test against an editable node needs one extra beat: `cmd-z` is bound on
`"Canvas && !Input"`, so while an editor holds focus the canvas never sees it. Close the editor
(escape) before pressing it.

Do **not** verify the app by sending system-wide keystrokes (`osascript`) to the running binary:
they land in whatever app is frontmost. That happened once during M1 and is why the headless
harness exists.

## 3. Manual feel-test

`cargo run -- --workspace <ws> --connection <conn>` and the checklist in `status.md`.
Screenshots via `screencapture -x` are fine for a visual check of a fresh launch.

## Conventions

- Views that tests must reach register with `.id("…").test_support()` before `.track_focus`.
- Constructors that tests need take the data directly (`WorkspaceView::with_document`) so no
  test touches `~/peek`.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` must pass;
  the lint set is strict enough that both are part of "tests green".

## Asserting on a component you cannot name

`window.find(id)` only resolves ids an element actually registers, and `within(outer)` scopes
the search but invents nothing. Some gpui-component elements register no id of their own:
`Editor` appears in the tree only as a path segment (`gpui_component::input::editor::Editor`)
with an unstable `NamedInteger("input", …)` leaf below it, so neither `find` nor `within`
reaches its bounds.

The trap is that a wrapper you *do* control is reachable — and useless. The Query node's editor
lays out `h_auto` unless given `.size_full()`, rendering only the rows that fit its intrinsic
height and clipping the rest; the wrapper around it fills its parent either way, so a bounds
assertion one level out passes whether or not the bug is present. A test that cannot fail is
worse than no test, because it reads as coverage.

When the element is unreachable, assert on **state** instead of geometry, from a
`#[gpui_kit::test]` inside the module that owns it, where the private entity is in scope.
`EditorState::visible_row_range()` is the discriminating observable here: `0..3` of five rows
with the bug, all five without it. `node/query/mod.rs`'s `layout_tests` is the worked example.

Always check a new regression test by reverting the fix and watching it fail. This one was
written twice before it earned its place.
