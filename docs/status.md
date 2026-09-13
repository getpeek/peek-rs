# Status

Last updated: 2026-09-12.

## Milestones

| # | Milestone | State |
|---|---|---|
| M1 | Load a real document, placeholder node shells, pan / zoom / camera flights, keymap + command palette | Done (headless tests green; user feel-test pending) |
| M2 | Node shell: selection rings, click / shift / marquee selection, drag, entry animation | Done except the entry animation: nodes are hit-tested by region (header / body / resize); the header drags, the body selects without dragging unless cmd is held (which drags from anywhere on the card), and the edges and corners resize within a screen-constant grab band |
| M3 | Themes: six Peek themes, gpui-component projection, picker with live preview | Done (syntax colours, picker swatches, dock icon deferred) |
| M4 | First real nodes, document mutations, undo, autosave; flip persistence to read-write | Done for seven kinds (Text, Variable, Draw, BarChart, TableDefinition, QueryError and now Query) plus the foundation: mutation API, per-page undo, debounced autosave, `--write` flag, per-kind body seam |
| M5 | peek-db: connections, SSH tunnels, results sidecar, schema, import | Done except import |
| M6 | peek-mcp bridge over the mutation API, peek-acp agent node | Not started |
| M7 | peek-multiplayer with an event-sink trait replacing Tauri's `AppHandle` | Not started |

Canvas features that slot in between milestones and are not started: regions and wayfinding,
minimap, page search, page tabs in the title bar.

### M5, in pieces

Done, and verified without a database:

- **`peek-db`**: sqlx Postgres and MySQL drivers, an `Engine` derived from the URL scheme with its
  identifier quoting, schema introspection, and the `UPDATE`/`DELETE` builders the Result node's
  inline editing will run. Four deliberate deltas from the reference, each a fix rather than a
  port: a **columnar `ResultSet`** instead of repeating the column name and type in every cell of
  every row; a **typed error enum** (a query error belongs next to the query, a connection error
  belongs in the connection UI — `Result<_, String>` cannot tell them apart, and the reference
  additionally flattens every schema failure to a fixed string, discarding the driver's message);
  **`Cell::Undecodable` distinct from `Cell::Null`**, so a column of unreadable bytes no longer
  reads as a column of NULLs; and **arms for the types the reference has none for** (arrays,
  `bytea`, `interval`, `time`, enums), which fell through a raw-bytes-to-UTF-8 arm and silently
  became `null` whenever sqlx negotiated the binary format.
- **SSH tunnels with real host-key verification.** The reference's `check_server_key` returns
  `Ok(true)` for every key — no `known_hosts` check at all — and the database credentials travel
  through that tunnel. `HostKeyPolicy` is trust-on-first-use by default and **always** refuses a
  key that has *changed*, which is the case that means someone is in the middle. Only russh's
  `Error::KeyChanged` counts as changed: the other `Err`s out of `check_known_hosts` (no home
  directory, a non-UTF-8 byte in `known_hosts`, an unparsable matching line) mean the file could
  not answer, and reporting them as an active MITM sent the user hunting for an attacker who was
  not there.

  This was written in M5 but had **never run**: `WorkspaceView::switch_connection` rebuilt the
  document, canvas and autosave and never reconnected the database, and the only tunnelled
  connections in `settings.json` are not the startup default, so `SshTunnel::open` was
  unreachable. Switching now reconnects, which also means `SshTunnel::close` has to exist:
  `local_port` is a fixed number two connections routinely share, `Drop`'s `abort()` only
  schedules the accept task's cancellation, and the next tunnel bound the same port microseconds
  later. `close` awaits the accept task so the listener is gone before the rebind.
- **The results sidecar.** `ResultSidecar`/`ResultsFile` read and write `<connection>.results.json`,
  gated by `PersistenceMode` and guarded by the same mtime check the document uses. Rows are
  session state on `peek_canvas::Document` behind their **own** revision counter, so a query never
  rewrites the document and a document edit never rewrites megabytes of rows; they never reach a
  history snapshot, for the reason edge selection does not. Pre-sidecar inline rows are lifted on
  load, with the sidecar winning on conflict — the merge order `useLoadDocument.ts` uses. Unlike
  `useAutoSaveResults.ts`, the sidecar **is flushed on quit**: the reference flushes the document
  but not the results, so its last query before a connection switch is lost.
- **The tokio↔gpui seam.** `CLAUDE.md` forbids a tokio runtime in UI code and sqlx is built on
  one, so `peek_db::Session` owns the runtime on its own threads and every call hands back a
  `oneshot::Receiver` — a plain future with no reactor of its own, which gpui's executor awaits
  like any other. One connection behind one mutex, as the reference has it.

Verified: `crates/peek-document/tests/sidecar_real_file.rs` parses and round-trips all seven real
sidecars under `~/peek` (3 KB to 13 MB), and `cargo run -- --workspace plock --connection
production` logs `432 result sets loaded`.

**Connection switching** replaces the database, not just the document. `Database::connect`
clears the language server's schema and the dialect before it opens anything, so a failed connect
cannot leave the previous database's tables completing — the symptom that found all of this. The
connection pill observes the `Database` global (nothing did before, so `Scope::connected` and the
Run button rendered whatever was true at the last unrelated repaint) and shows a failure as a
warning glyph plus the driver's or tunnel's own message in its popover, where the reference puts
it.

**Query execution** is wired to the canvas. `Database` is a gpui global holding the session; it
connects on startup from `settings.json` and introspects the schema into `peek_lsp`, which is what
finally lights up table and column completions. `Query::Run` (`meta-enter`, bound on the node
context so it fires while the SQL editor has focus) runs the selected query; the footer's Run
button is live and reads a real `Scope::connected`.

What happens on a run, all of it ported from `executeQueries.ts`:

- Variables are collected from the variable nodes feeding the query, walked in **edge-id order**
  so two nodes defining the same name resolve the same way every run, and list values join with
  `", "` after blank lines are dropped (the list editor's trailing newline would otherwise put a
  dangling comma inside an `IN (…)`).
- **An undefined variable becomes a `query-error` node without the database being asked**, which
  is what stops `delete from t where id = @missing` running with the `@missing` still in it.
- The **unbounded-write gate** (`TRUNCATE`, or a `DELETE` with no `WHERE`) is checked **before**
  substitution — a `WHERE` that only appears once a variable resolves still counts as unbounded,
  which is the safe way round — and needs one confirmation, the reference's red "Run unbounded"
  two-click rather than a modal. Any edit to the query withdraws it. Unlike the reference, the
  gate sits *below* the entry point rather than on the button, so the live tick and later the MCP
  tools cannot walk around it.
- The result lands at `x + width + 50`, sized from its columns (`UUID 440`, `TIMESTAMP 280`,
  `NUMERIC 130`, else `250`, floored at 200; height `min(rows * 50 + 140, 1500)`). Re-running
  updates the same node, because the id is derived from the query's; **a live query's node is not
  resized**, so polling never fights a manual drag. Failures place a `query-error` node connected
  **backwards** (error → query), and a success deletes it again.
- Placing a result is **one undo step**, not four: `Document::transaction` suppresses the nested
  `begin` calls that insert, connect, write data and clear the error would otherwise each make.
  There is a regression test, checked by reverting the guard and watching it fail.
- The **live tick** polls at `liveIntervalMs` (10 s), skipping while the editor has focus, while a
  run is in flight, and for anything that is not a plain `SELECT`.

Rows never enter the undo snapshot or the document revision — they have their own counter and
their own file.

Verified against a real database: `crates/peek-db/tests/live_postgres.rs` (seven tests) and an
end-to-end `#[gpui_kit::test]` in `execution.rs` that connects, runs a query node and asserts the
result node and its rows. Both are opt-in on `PEEK_TEST_DATABASE_URL`, so the default suite stays
hermetic; `cargo run -- --workspace Plock --connection local` logs `connected to local
(PostgreSQL)` and `schema has 129 tables`.

Left in M5: **import** (csv/json → temp table), and the multi-statement fan-out — `executeQueries`
takes a list of queries, but the Run button passes one, and the callers that pass several (follow
references, fork) arrive with the Result node.

### The Result node

The table renders. `node/result/` holds a `TableDelegate` over the rows on the session
`Document`, and `TableState` owns virtualisation, the visible range and scrolling — a 6,515-row
result builds **12 rows**, which `a_large_result_only_builds_the_rows_it_shows` pins.

Two deliberate differences from the reference, both forced by `DataTable` virtualising with
`uniform_list`, which measures the first row and assumes the rest match:

- **Rows are a uniform 34 world units.** The reference measures every row, because a JSON cell
  renders its whole pretty-printed tree inline; `ROW_HEIGHT` there is only the estimate.
- **JSON cells show a one-line summary** (`{…} 3 keys`, `[…] 7 items`) and will open in a detail
  panel rather than growing the row.

Column widths are the reference's: the longest of the header and the first 30 values at 7.2 px a
character plus 28 padding, clamped to 80–360, with an explicit width from `columnWidths`
overriding and uncapped, and the whole set scaled up when the columns do not fill the node.
Numbers are right-aligned in tabular figures; `NULL` is muted italic, `TRUE`/`FALSE` take the
blue and red the stylesheet gives them, and a `Cell::Undecodable` says "unreadable" rather than
passing for a NULL.

**The table sizes itself in pixels, so it is the first kind that needs the camera's zoom.**
Everything else is rem-based and scales for free inside `with_rem_size(base * zoom)`, but a
column width is a `Pixels`, and a body is built in `CanvasView::render` — before the rem scope it
is later laid out in — so `window.rem_size()` is still the base there. `NodeContext` carries
`zoom` for this.

**Wheel absorption is not free.** `CanvasElement` registers its wheel listener before the node
elements precisely so a node can absorb a scroll, but gpui's built-in scroll handling stops no
propagation and `DataTable` adds none, so without `has_room_to_scroll` a scroll would move the
rows *and* drag the canvas. The rule is `useScrollFallthrough`'s `canAbsorb`: absorb only while
there is room in the direction being scrolled, so reaching the end of the rows hands the gesture
back to the canvas.

**The canvas does not steal presses from a node, and pointer arbitration was never needed.**
That was the planned next milestone, on the reasoning that `CanvasElement`'s window-level
listeners run before any node element. They do — but they do not `stop_propagation`, so the node
element's handlers run straight after, and a full click on a cell selects that cell with the
canvas's listeners fully active. A probe test established this before any of it was built. The
canvas resolves the same press as a body hit and selects the node, which is what React Flow does
too, so clicking a cell selects the cell *and* the node.

What the canvas press *does* still own is the modifier rule: a plain press on a body never drags
or marquees, and cmd + press turns the card into a drag handle — the reference's `nodrag`, which
it drops while cmd is held.

The real blocker was **focus**, and it is fixed. The table owns a focus handle, and a handle dies
with its node: clicking a cell and then deleting the node left gpui focused on nothing, which
silently killed every canvas binding including `cmd-z`. `CanvasView::reclaim_focus` takes focus
back whenever the window is left focused on nothing, so it covers deletion by any route — the
Delete key, undo, MCP, a page switch. `deleting_a_focused_table_leaves_the_canvas_usable` fails
without it.

A finished column drag writes its widths back to the document in world units, so a resize
survives a re-run and a reopen.

**Selection is ours, not `DataTable`'s.** Its own selection is a single `Option<(row, col)>`, so
`selection.rs` holds the model — a rectangle of cells or a set of whole rows, never both, since a
copy has to mean one unambiguous thing. `cell_selectable`, `row_selectable` and `col_selectable`
are all **off** rather than left to paint a second, disagreeing selection. The gestures are the
reference's:

- A press on a cell anchors a rectangle; dragging extends it in any direction.
- A press on a column header takes the whole column; dragging across headers extends the range.
- **Shift** is the row gesture: shift-press toggles a row, shift-drag sweeps a band and keeps
  what was already selected. A sweep that moved is a range, not a toggle — that distinction is
  only knowable on release, which is why `release()` exists.
- Escape clears, and so does a press on the table's blank space.
- `cmd-c` copies TSV, where NULL is the empty string so a spreadsheet cell arrives blank.
- New rows clear the selection: a position only means something against the ordering it was
  captured in, and search will re-sort those positions by match score.

Two wiring rules this needed, both found by watching a test fail:

- **A cell press must `stop_propagation`.** The body clears the selection on a press that is not
  on a cell, and without the cell claiming its own press first, every click selected and then
  immediately cleared.
- **A cell press focuses the table.** Actions dispatch from the focused element *outward*, so
  `escape` and `cmd-c` only reach the node's handlers when the table — a descendant of the
  canvas, not an ancestor — holds focus. `CanvasView::reclaim_focus` covers the handle dying
  with the node.

The table's accessible label names the selection (`4 cells selected`), which is both what a
screen reader gets and the only stable observable a test has: `DataTable` registers no ids for
its cells.

**The toolbar and in-result find** are in. Three exclusive states, in the reference's own
precedence: the find bar while a search is open, the selection statistics while a numeric
rectangle is selected, and otherwise the meta row — a status dot, the row count, and a badge per
table the query reads (`peek_lsp::analyze_query`, recomputed only when the SQL changes). Chart,
Export and Pivot render **disabled with a tooltip naming what they wait for**, the convention
`canvas/toolbar.rs` already uses for tools with no command behind them.

Statistics follow `aggregate.ts`'s one load-bearing rule: **a single non-numeric cell and there
is no answer at all**. A number under a column of names is worse than no number. A numeric-looking
string only counts when its column is numeric, so a column of postcodes does not offer to average
itself, while `NUMERIC` — which rides as text to keep its precision — does.

Find is `cmd-f` (`Page::Search`, on the node's context), debounced 100 ms, threshold 0.5, rows
re-sorted by their best cell and matched cells tinted. **The scoring is ours, not fuzzysort's.**
gpui-component has no fuzzy matcher — its palette is a plain case-insensitive `contains` — so
rather than add a dependency, `search.rs` scores a subsequence on three things a database grid
cares about: an unbroken run beats a scattered match, a match at the start of a value beats one
in the middle, and a shorter value holding the match beats a longer one. The threshold is
calibrated so a scattered subsequence falls below it and a literal substring clears it easily.

Searching drops the selection, for the same reason new rows do: display positions only mean
something against the ordering they were captured in, and search re-sorts them.

**Keys and the value pane.** Columns are classified against the schema the language server
holds: a column something points at is a key, one that points somewhere is a reference, and the
name shapes (`^id$`, `_id$`) are only a tie-breaker for the queries whose columns cannot be
traced to a table — an aggregate, a join, a CTE. Headers carry a `PK`/`FK` tag and key values
take the yellow and blue the stylesheet gives them. One faithful oddity: the leading-column rule
is checked **before** outbound references, so a result keyed by `user_id` reads as a key even
though it also points at one — surprising written down, right on screen.

Double-clicking a cell opens its full value in a pane under the toolbar: JSON as a tree with the
reference's 36-character middle truncation and `123ch` badge, long text wrapped and scrollable,
NULL and an undecodable value each saying so.

**The pane is inside the node, not floating over it** — a deliberate reversal of the earlier
plan. gpui lays overlays out at rem 1 whatever the camera is doing, so a popover would not scale
with the node it explains, and `deferred` inherits the content mask in force where it was created
— which for a node body is `overflow_hidden`, so it would be clipped by the very table it is
explaining. The Variable node reached the same conclusion and expands its list editor inline.

Still to come: **follow-references** (clicking a reference chip runs
`SELECT … WHERE col = 'v' LIMIT 300` and fans new result nodes), which needs the multi-statement
fan-out `executeQueries` has and `execution::run` does not — it runs one statement from a query
node. Then per-character match highlighting (matched *cells* are tinted; the reference also
underlines the matched characters), the dashed ghost hover preview, inline editing, row deletion,
export, the context menus, pivot and chart sync. Keyboard cell navigation is absent in the
reference too, and went out with `DataTable`'s own selection; it is worth adding back on our
model.

### Keyboard navigation

Done: `g` opens Vimium-style jump labels over every visible node, `cmd-arrow` walks the selection
through a 45° cone, and `enter` drops into the selected query's editor (escape backs out, a second
escape clears the selection). See `docs/canvas.md`, "Keyboard navigation". The reference's
`meta-[` / `meta-]` query cycling is not ported.

### Edges

Done: the floating bezier of `FloatingEdge.tsx` — both endpoints recomputed every frame from
where the centre-to-centre line crosses each node's box — painted between the dot grid and the
nodes, tinted by the kind it **points at**, and thicker and brighter while either of its nodes is
selected. A press within ten world units of a curve selects the edge (clearing the node selection,
as React Flow's store does, and vice versa); `backspace` deletes nodes and edges in one undo step.
Edge selection is a second session-only `BTreeSet<EdgeId>` on `Document`: it must not reach a
history snapshot, since `Snapshot` compares by value and selecting would otherwise become an
undoable edit.

Not done: dragging a new connection into being (the variable node's four source handles are still
absent for that reason), and the `.flowing` / `.query-live` marching ants, which need live queries
and so wait for M5.

### The draw tool

The Draw node renders; the tool that creates one does not, and `Tool::Draw` is unregistered.
The reducer work is specified — read from `useDrawTool.ts` rather than inferred:

- Arm with the existing `Input::ArmTool(Some(NodeType::Draw))`, but branch to a new
  `Interaction::Drawing { samples: Vec<Point> }` rather than `Placing`. **Draw mode is sticky**:
  `useDrawTool` never clears place mode, so a committed stroke leaves the tool armed for the
  next one until escape. Place tools are one-shot; this is the opposite.
- Samples are stored in **world** space, converted on arrival. The reference keeps client
  coordinates and converts the batch at commit, which is only equivalent because its camera
  cannot move mid-stroke; converting on arrival keeps the camera off the commit path and stops
  a wheel-pan mid-stroke from shearing the drawing.
- No new `Input` variants, and **no `pressure` field**: gpui's `MouseMoveEvent` does not carry
  one (`MousePressureEvent` is the force-click stream, not stylus pressure), and the reference
  records `e.pressure || 0.5` — a constant `0.5` for every mouse stroke. Peek renders with
  `simulate_pressure`, deriving width from sample spacing, so a constant is faithful.
- While drawing: `Down` on `Button::Left` only and **ignoring `on_node`** — that is the
  analogue of the reference's `stopPropagation`, so a press over a node starts a stroke rather
  than selecting or dragging it. `Move` pushes every sample with no drag threshold. `Up`
  commits when there are at least two samples, then clears them and stays armed.
- One new `Effect::PlaceDrawing { points: Vec<Point> }`, applied through a
  `Document::create_drawing(&[Point])` so the node, its data and its undo entry are one
  mutation. Geometry: `PADDING = stroke_width * 2 = 8` world units, `position = min - 8`,
  `size = extent + 16`, `points[i] = [w - min + 8, 0.5]`, `stroke_width = 4.0`.
- **Colour is a trap.** `NodeKind::empty(NodeType::Draw)` yields `"white"`, which is
  `makeNode`'s palette default; `useDrawTool` overrides it to `"var(--pk-fg)"`. The commit must
  set it explicitly or every drawn stroke is white in every theme.

Window-level listeners already give the pointer capture the reference gets from
`setPointerCapture`.

## Chrome

The two floating canvas panels are deliberately *different* surfaces, as `Toolbar.css` has them:

- **The tool palette** (`canvas/toolbar.rs`) is a card — a half-opaque `node_bg` (the derived
  `PeekTheme::chrome_bg`) behind a full `node_border` hairline, `radius_card`, 5 px padding, 4 px
  gaps, 30 px buttons with 16 px glyphs, `.sep` rules after Select and Draw.
- **The zoom cluster** (`canvas/hud.rs`) is a pill — transparent fill, a *half-alpha* hairline,
  `radius_pill`, 3 px padding, 1 px gaps, 26 px buttons with 14 px glyphs, and a 44 px column of
  tabular figures. It reads as a readout, not a second tray of tools.

Both are pinned in pixels rather than rems because the reference's chrome does not scale with a
root font size: at Peek's 13 px rem the nearest component size step renders a 26 px button where
the palette wants 30. Neither gets `backdrop-filter` — gpui's only blur is `BoxShadow::blur_radius`,
so the canvas shows through unblurred.

Locking the camera disables the four zoom controls, as the reference does; the lock itself stays
live, since it is the way back out. The reference floats the lock bottom-right and shows it only
while locked, which leaves no visible way to lock — here it closes the cluster it governs.

The title bar's connection pill is tinted by the connection's own `color` from `settings.json`,
parsed by `peek_config::DatabaseConnection::rgb` (hex and `hsl()`, the two forms the apps write).
The workspace list is snapshotted once at startup, as the reference's `useGetConfig` does, so a
connection added in the TypeScript app needs a restart to appear — and rendering the pill never
touches the disk.

The window minimum is 760 × 480 rather than the old 640 × 400: the bottom chrome is two panels,
one pinned left and one centred, and below about 750 px the centred one runs into the other.

## Persistence is opt-in

The app reads the real `~/peek/settings.json` and `~/peek/workspaces/<ws>/<conn>.json`, and
writes only when launched with `--write`. `peek_config::PersistenceMode` still gates every save
path (`PeekConfig::save_to_disk`, `DocumentFile::save`, theme commit) and still defaults to
`ReadOnly`, so the Tauri app can keep running and autosaving alongside.

When writing is enabled, `DocumentFile` guards the shared files two ways: every save is atomic
(temp file plus rename), and a save is refused with `StorageError::ChangedOnDisk` when the
file's modification time is not the one this session last read or wrote — autosave then stops
rather than overwriting the other app's work. The first save of a session also copies the
previous contents to `<connection>.json.bak`.

Autosave itself is a 3 s debounce off the session document's revision counter, restarted on
every edit and flushed on quit, matching `useAutoSaveDocument.ts`. Selection changes do not
bump the revision, so they never schedule a write.

## Run and verify

```
cargo run -- --workspace <name> --connection <name>   # defaults to the first workspace/connection in settings.json
cargo run -- --write                                  # enables autosave; read-only otherwise
cargo test --workspace                                # 455 tests, a few seconds after the first build
cargo test -p peek-config -p peek-document -p peek-canvas   # the gpui-free crates, seconds
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`RUST_LOG=debug` prints camera flights and theme applications.

What to try in the running app: two-finger scroll pans, pinch and cmd+wheel zoom about the
cursor, middle/right drag and space+drag pan, `cmd-0` reset (200 ms), `cmd-shift-0` fit all
(300 ms), `cmd-=`/`cmd--` step, `cmd-shift-l` camera lock, `cmd-shift-[`/`]` page switch,
`cmd-a` / `escape`, `cmd-p` palette, palette → "Change theme".

## The feel-test this milestone owes

Every node in M4 was verified through headless `#[gpui_kit::test]` dispatch against vendored
sources. Nothing in that harness reads rendered pixels, so these are open until someone runs
`cargo run` and looks:

- **The completion popover on a node near the edge of the viewport.** It is a `deferred` draw,
  so it inherits the content mask in force where it was created — the node body clips with
  `overflow_hidden`. If it turns out to be cut, the fix is to stop clipping the query body, not
  to move the popover.
- **Draw at several zoom levels.** Its stroke recovers the zoom factor itself rather than through
  the rem scope, and that path is only unit-asserted — never driven through a real zoomed frame.
- **Text's font size.** It is the node's height × 0.62, so a default 280 × 140 node renders at
  86 px and a 300 × 200 one at 124 px. Faithful to the reference, and startling until seen.
- **Variable at its 220-unit minimum width**: whether the 40 % name column leaves the value
  column usable, whether `[ ]` reads as an array toggle, and whether the inline list editor
  crowds a four-row node when expanded.
- **Tooltips at high zoom**, which lay out at rem 1 and so do not scale with the node.
- **BarChart with two series**, the case that motivated the hand-composed plot.

## Known gaps and quirks

- Three kinds still show the placeholder body: `ResultInsertForm`, Agent and Activity, each
  waiting on peek-acp/peek-mcp (M6).
- **The Query node runs**, and its result shows its rows. `Query::Run` (`meta-enter`) executes
  against the connection opened at startup, places the result or a `query-error` node, and polls
  when live is on.
- **Query completions and diagnostics now see the schema.** `Database` fills
  `peek_lsp`'s `SchemaIndex` once a connection answers (129 tables on the local Plock database),
  so completions offer tables and columns and `diagnose` flags unknown ones. Before a connection
  answers it is empty, which is deliberate: the reference returns nothing rather than painting
  every table red while the schema loads.
- **Clicking into a query editor puts the caret at the end of the text, not where you clicked.**
  The earlier diagnosis here — that the canvas's window-level listeners swallow the press — is
  **wrong**, and the Result node disproved it: the canvas does not `stop_propagation`, so node
  elements do receive their own presses (a click on a table cell selects that cell). Whatever
  keeps the editor from placing its caret is local to how the node hands it focus, on mouse *up*,
  after the mouse-down that would have positioned the caret has already passed.
- Undo covers nodes, edges and regions per page. Page creation and deletion are not undoable,
  matching the TypeScript app.
- **The Variable and BarChart nodes still label buttons that should be icons.** The icon source
  they needed now exists: `crates/peek-ui/src/assets.rs` embeds the named glyphs (including
  `Trash` and `Brackets`) via `icon_assets!` and falls through to the default bundle, so those
  two nodes can adopt icons whenever someone passes through them.
- **Node bodies can scroll now.** `CanvasElement::paint` registers its wheel and pinch
  listeners *before* the node elements paint, so gpui's reverse bubble order offers a wheel to
  the hovered node first; gpui-base's editor stops propagation only when it actually scrolled,
  which is `useScrollFallthrough`'s rule, so anything else falls through and pans the canvas.
  cmd/ctrl + wheel is taken in the capture phase instead, so a zoom is never swallowed by an
  editor under the pointer. The Query editor is the first kind to use it; M5's virtualised
  Result table needs nothing further.
- **Shifted non-letter bindings are spelled as the character they type** — `meta-shift-0` binds
  `cmd-)`, `meta-shift-[` binds `cmd-{` — because macOS clears the shift flag for those keys
  (`docs/commands.md`, "Keystroke translation"). The shifted table is US ASCII; a layout that
  types `)` elsewhere would need gpui's `key_equivalents`, which Peek does not use yet.
- Mouse-wheel (non-trackpad) viewport commits use a 140 ms quiet-period timer.
- The title bar carries the page tabs and the connection picker; no collaborate button yet.
  **The picker has no keyboard trigger.** The reference binds bare `p` to `ConnectionPicker::Open`
  (the id is reserved in `settings.schema.json`), but gpui-component's `DropdownMenu` exposes only
  `on_open_change`, not a controlled `open`, so a keystroke cannot open it. Doing so means owning a
  `Popover` and its `PopupMenu` by hand, at which point the command registry entry can land too.
- **Switching connections does not reconnect the database.** It reloads the document, its pages,
  its rows sidecar and its autosave, but `connect_to` is only called at startup, so the session
  keeps talking to the connection it opened then — queries run against the wrong database and the
  schema behind completions is stale. `WorkspaceView::switch_connection` needs the same
  `Database::connect` call `new` makes.
- No FPS overlay (`gpui-fps` is not a published crate for this gpui-pre version).
- Fonts: the theme names "Monaspace Krypton" and relies on it being installed; bundling the OTFs
  via `cx.text_system().add_fonts` is deferred.
- A user keymap entry naming a command id not yet in the registry (e.g. `Page::New`) is logged and
  skipped at startup until that command lands.
