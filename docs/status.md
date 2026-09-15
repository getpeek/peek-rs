# Status

Last updated: 2026-09-13.

## Milestones

| # | Milestone | State |
|---|---|---|
| M1 | Load a real document, placeholder node shells, pan / zoom / camera flights, keymap + command palette | Done (headless tests green; user feel-test pending) |
| M2 | Node shell: selection rings, click / shift / marquee selection, drag, entry animation | Done except the entry animation: nodes are hit-tested by region (header / body / resize); the header drags, the body selects without dragging unless cmd is held (which drags from anywhere on the card), and the edges and corners resize within a screen-constant grab band |
| M3 | Themes: six Peek themes, gpui-component projection, picker with live preview | Done (syntax colours, picker swatches, dock icon deferred) |
| M4 | First real nodes, document mutations, undo, autosave; flip persistence to read-write | Done for seven kinds (Text, Variable, Draw, BarChart, TableDefinition, QueryError and now Query) plus the foundation: mutation API, per-page undo, debounced autosave, `--write` flag, per-kind body seam |
| M5 | peek-db: connections, SSH tunnels, results sidecar, schema, import | Done except import |
| M6 | peek-mcp bridge over the mutation API, peek-acp agent node, local Ollama backend | Done |
| M7 | peek-multiplayer with an event-sink trait replacing Tauri's `AppHandle` | Not started |

Canvas features that slot in between milestones and are not started: regions and wayfinding
(the model and its mutations landed with M6's tools, but nothing draws them yet), minimap,
the history panel, and the Activity node's running-query list.

## The command palette port

The reference's ~40 palette commands are ported. The registry went from 38 entries to 55,
split one file per group under `commands/registry/` — a single array was a merge conflict
every time two features landed at once. New in this batch: `Query::{RerunAll,RerunSelected}`,
`Export::{Csv,Json}`, `Page::{GoTo,OpenPicker,SelectPreviousQuery,SelectNextQuery}`,
`Page::Search` (now page-wide, not just in-result), `Result::Pivot`,
`View::{Organize,Schema}`, `Zoom::FitSelectionAndLock`, `Edit::{Cut,Paste}` and a canvas-level
`Edit::Copy`, `Settings::{TogglePageDisplay,ToggleCommandPaletteButton}`, `Help::Keymap`,
`App::About`, `ConnectionPicker::Open`.

Deliberate divergences from the reference, each for a stated reason:

- **Export writes a header-only CSV / `[]` for an empty result.** The reference's `toCsv`/`toJson`
  read `result[0]` and throw. Divergence in the safe direction.
- **The CSV header is joined unquoted**, matching the reference, so a column named `a;b` produces
  broken CSV. Kept on purpose: byte-identical exports between the two apps beat a silent local
  fix, for the same reason the on-disk formats are frozen.
- **Export filenames are always the SQL slug.** The reference optionally asks Ollama for a nicer
  name; an export never waits on a model that may not be running.
- **Rerun walks queries left-to-right by x**, where the reference uses React Flow insertion
  order. That order is not a decision there — it is `rf.getNodes()`'s array order surfacing — and
  it is unstable: delete a query, re-add it, and the page reruns in a different sequence with no
  visible cause. Since runs are staggered rather than concurrent, the order is observable, so a
  predictable one wins. **It is a visual ordering, not a dependency ordering**: queries that feed
  each other through variable nodes are not sequenced by it, and two queries at the same x resolve
  by `total_cmp` — deterministic but arbitrary. Nothing in the rerun path depends on ordering for
  correctness today; if sequencing ever becomes load-bearing, x position is the wrong input.
- **The schema page is laid out once and stops.** A tick is a document mutation, so a perpetual
  simulation means autosave never quiesces and the undo coalescing window never closes.
  Schema edges are persisted, which *is* the reference's shape — `edgesAtom`'s setter writes into
  `doc.pages[…].edges`.
- **One key, two meanings, chosen by focus.** `Edit::Copy` is a single action on
  `CANVAS_NOT_TYPING`: the canvas copies the selected nodes, and a focused result table copies
  its cells as TSV instead, winning on depth.
  `copying_in_a_focused_result_does_not_also_copy_the_node` pins it — and carries a positive
  control, because "no node was pasted" would otherwise pass just as well if pasting were broken.
  Switching the canvas handler to `capture_action` (root-to-leaf) makes it fail, which is the
  regression it exists to catch.
- **`⌘F` has no dead zone.** The reference arbitrates it with two listeners whose guards
  (`selected && !pivoted`, and `nothing selected`) are not complementary, so a selected pivoted
  result leaves the key doing nothing. Here the discriminator is focus, so exactly one handler
  always runs.
- **The keymap modal derives its rows from the registry** rather than a hand-kept list. The
  reference duplicates its descriptions from markdown and has already drifted — `Page::OpenPicker`
  is in its `keymap.rs` and in-app help but missing from its `docs/keymap.md`.

Two honest gaps in the batch's own testing, recorded rather than papered over:

- **`window.refresh()` in the settings toggles is temporary**, standing in for an
  `observe_global::<Settings>` subscription on `WorkspaceView`. Its repaint has **no test**:
  removing the call leaves the toggle test passing, because the headless harness re-renders every
  frame regardless. A test that cannot fail is worse than none, so this is a note instead of one.
- **The connection picker's write paths are unit-tested but not driven end to end**, because
  making `WorkspaceView::with_document` writable in a test would aim `DocumentStore` at the real
  `~/peek`. Threading a base directory through the view is the fix, whenever a second consumer
  wants it.

Not ported, each blocked on a feature rather than on the command: the region commands (group,
ungroup, the two AI groupings, the regions toggle), the minimap toggle, "Show running queries",
"Show history", host/join session, automatic query labels, and `Tool::LassoSelect` — which is a
freehand selection tool, not a command. Registering any of them would put a row in the palette
that does nothing.

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

**Editing and deleting rows.** Double-clicking a short value opens an editor in the cell; Enter
commits, escape cancels. A JSON object or anything past 36 characters opens in the value pane
instead, which is the only place either fits. A commit resolves the editable table and its
primary keys, builds `UPDATE "t" SET "c" = <lit> WHERE "pk" = <lit>`, runs it, and then **re-runs
the query node behind the result** rather than re-issuing the SQL by hand — that re-resolves the
variables, re-places the rows and clears any error through the one path. A failure leaves the
editor open with the reason in a strip under the toolbar; the reference floats that under the
cell, which a clipped, virtualised table has nowhere to do.

Deleting is the same machinery over a row selection, behind a confirm dialog that says it cannot
be undone. The affordance only appears when rows are selected **and** the result is writable, so
the one irreversible action in the table never sits there inviting a stray click.

**None of this is undoable.** These statements change the database, not the document, so the
canvas history never sees them. That is why every refusal — not a single-table `SELECT`, no
primary key, a result that does not show the key columns — is a named `NotEditable` checked
*before* anything is sent, and why the statement is pinned to one row by construction. The
unbounded-write gate has nothing to say about it: it only flags `TRUNCATE` and a `DELETE` with no
`WHERE`, and neither can be built from here.

A cell's handlers run inside a `TableState` update, so opening the editor is deferred with
`window.defer`: reading that state back from there is a re-entrant borrow, which gpui turns into
a non-unwinding panic that aborts the process. See `docs/canvas.md`, "Editable nodes".

Verified against a real database in a `CREATE TEMP TABLE`, which lives on the connection and
disappears with it: an update changes exactly one row and leaves its neighbour alone, a delete
removes only the row it names, and a value containing `o'brien; drop table x` is stored verbatim
rather than executed.

**Following a reference.** A cell whose column really points somewhere — or is really pointed at
— is a link: pressing it claims the press so the table does not also start a selection, and
clicking it asks the database for the rows on the other side and fans them onto the canvas beside
the result they came from, with an edge saying where they came from. Inbound wins over outbound,
because on a primary key "what points at this row" is the useful question.

Only a **schema-backed** reference is a link. A `*_id` column with nothing behind it is tinted,
because the name says what it is, but inert: there is no target to follow, only a name that looks
like one. The reference behaves the same way — without the schema its chip gets no click handler.

This needed `execution::run_queries`, the multi-statement fan-out `executeQueries` has and the
single-statement `run` did not: one query per foreign key, each placed and each failing on its
own. Two fixes to the reference's query, which interpolates
`` `... WHERE ${ref.column} = '${value}'` `` directly: identifiers go through
`Engine::quote_identifier`, so a table called `order` parses, and the value goes through the
literal formatter, so a key holding an apostrophe cannot end the literal early.

Still to come: per-character match highlighting (matched *cells* are tinted; the reference also
underlines the matched characters), the dashed ghost hover preview, export, the context menus,
pivot and chart sync. The editors are single-line fields for now: the reference gives booleans a
three-state picker and long text a growing textarea, and `@variable` autocompletion inside a cell
is not ported. Keyboard cell navigation is absent in the
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

Done. `Tool::Draw` is registered on `d`, so the toolbar's pencil lights up with its badge and
the palette lists it — `canvas/toolbar.rs` derives both from the registry and needed no edit.

The reducer branch is `Interaction::Drawing { samples }`, and `gesture::on_drawing` is offered
every input before the ordinary paths. That is what the reference's `stopPropagation` buys: no
press reaches pan, marquee, drag or resize, so a stroke started over a node draws rather than
moving it. Wheel and pinch are handed back, so two-finger scroll and pinch zoom keep working
while the pen is armed. **Draw mode is sticky** — `useDrawTool` never clears place mode, so a
committed stroke leaves the tool armed until escape, the opposite of every other place tool.

Three deltas from the reference, all in `docs/canvas.md`, "The draw tool":

- **Samples are world units, converted on arrival**, so a pan mid-stroke moves the ink with the
  canvas instead of shearing it. The reference can only convert the batch at commit because its
  camera cannot move during a stroke.
- **No `pressure` field.** gpui's `MouseMoveEvent` carries none (`MousePressureEvent` is the
  force-click stream, not stylus pressure), and the reference records `e.pressure || 0.5` — a
  constant for every mouse stroke. `simulate_pressure` derives width from sample spacing, so
  the stored `0.5` is never read back.
- **A non-left press does nothing.** The reference returns before its `stopPropagation`, so
  React Flow's `panOnDrag={[1, 2]}` still pans mid-mode; matching that would mean teaching
  `Panning` to return to `Drawing` on release. The loss is middle-drag, right-drag and
  space-drag until escape.

One addition the reference has no analogue for: **a sample identical to the one before it is
dropped**, which `getStrokePoints` does anyway. It is also what keeps a stationary click from
committing a degenerate one-point dot, since a synthetic pointer stream can emit a move that a
real stationary click never would.

`Document::create_drawing` is the commit, in one `transaction` so the node and its data are a
single undo step — undoing a stroke never leaves an empty drawing behind. Geometry is the
reference's: the samples' extent inset by `PADDING = stroke_width * 2 = 8`, points relative to
the origin, `stroke_width = 4`. **Colour was the trap**, and it is set explicitly:
`NodeKind::empty(NodeType::Draw)` yields `makeNode`'s palette default of `"white"`, which would
be invisible in a light theme.

The live preview is `LiveStroke.tsx`: pane-relative screen points painted above the nodes at
`stroke_width * 4 * zoom`, through the same `node::draw::tessellate` the committed node uses,
so the shape cannot change at the moment the pen lifts. It reads `peek_canvas`'s exported
`DRAW_COLOR` and `DRAW_STROKE_WIDTH`, so preview and commit cannot drift. `CanvasView::reduce`
returns early when a gesture produces no effects, which is the whole of a stroke until the pen
lifts — so a non-empty `stroke_world()` now earns its own repaint.

Covered by four reducer tests, two on `create_drawing`, and three `#[gpui_kit::test]`s in
`crates/peek-ui/tests/draw.rs` driving real pointer dispatch: the padded geometry, stickiness
across two strokes and escape, and a stroke drawn across a node's header leaving it in place.
Nothing in that harness reads pixels, so **the preview itself is a feel-test item** below.

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
It is a trigger, not a menu: it dispatches `ConnectionPicker::Open` through the canvas focus
handle, the same action bare `p` is bound to, and holds a deeper fill while its panel is up.

**The picker is a hand-owned panel** (`title_bar/picker/`), anchored under the pill at the
reference's own 460 px with a full-window transparent scrim behind it. Not `DropdownMenu`, which
has no controlled `open`; and not `Popover`, which *does* — the earlier note here was wrong about
that — but whose `appearance(false)` also disables outside-click dismissal and whose `trigger`
forces `Selectable` onto a pill that already hand-builds its hover surface. Owning it outright
also gives every row a `test_support` id, which a `PopupMenu` never had.

What it does, all of it ported from `WorkspaceList.tsx` and `WorkspacePopover.tsx`: fuzzy search
across five keys (workspace, connection, user, host, database, so typing a workspace surfaces its
connections), a cursor the arrow keys walk with Enter to switch, collapsible workspace groups that
auto-expand when the cursor enters them, the `SSH` badge, the `user@host` line, per-character
match underlines in the connection's own tint, and push-navigation into forms for adding, editing
and removing both connections and workspaces.

Three deliberate differences. **Rows are gathered under their workspace in cursor order**, so the
sequence read top to bottom is the sequence Enter walks — the reference groups for display but
keeps a flat score-ordered cursor, so its arrow keys can jump around the panel. **Renaming moves
the canvas**: a connection's document is keyed by name, and `DocumentStore::rename` moves it and
its rows sidecar before the config commit, where the reference orphans both. And the per-row "…"
popup is **inline hover buttons** instead, because a menu there is an overlay opened from inside
an overlay, and the reference's menu items only open the same form anyway.

Removing a connection takes only its `settings.json` entry; the canvas stays on disk, so re-adding
the name gets it back. Every write is gated on `PersistenceMode` — without `--write` the forms
open but Save and Duplicate are disabled and say why. `Session::probe` backs a Test button the
reference has no equivalent for: it opens a throwaway connection without locking the live session's
`Inner`, and forces `local_port: 0` so probing a tunnelled connection cannot collide with the live
tunnel's fixed port.

The workspace list is no longer snapshotted at startup. `Settings` is a gpui global and
`WorkspaceView` observes it, so a connection added in the picker appears in the pill immediately —
one added by the TypeScript app still needs a restart, since nothing watches the file.

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
cargo run -- --performance                            # level of detail on: distant nodes drop their bodies
cargo test --workspace                                # 676 tests, a few seconds after the first build
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
  The live preview compensates for zoom the other way round, by hand, so the two have to agree:
  the ink under the pen and the stroke left behind should be the same weight and in the same
  place at 0.4 and at 2.5.
- **Text's font size.** It is the node's height × 0.62, so a default 280 × 140 node renders at
  86 px and a 300 × 200 one at 124 px. Faithful to the reference, and startling until seen.
- **Variable at its 220-unit minimum width**: whether the 40 % name column leaves the value
  column usable, whether `[ ]` reads as an array toggle, and whether the inline list editor
  crowds a four-row node when expanded.
- **Tooltips at high zoom**, which lay out at rem 1 and so do not scale with the node.
- **BarChart with two series**, the case that motivated the hand-composed plot.

## Canvas frame cost

Zooming out over result nodes used to drop frames. Four costs were being paid per visible result
node per frame, and one by every node with text:

1. `result::body` deep-cloned the whole `ResultSet` — every `Cell::Text` string, every
   `Cell::Json` tree — and `ResultDelegate::adopt` then deep-compared it to answer "unchanged".
   The sidecar now holds `Arc<ResultSet>`, so the clone is a refcount bump and the steady-state
   comparison is `Arc::ptr_eq`. The delegate's second full copy went away with it. The value
   comparison is still there, but only behind a pointer miss, i.e. once per query run rather
   than once per frame — a **live** query re-runs every ten seconds and usually gets identical
   rows back, and treating that as a new result would drop the user's selection twice a minute.
2. The toolbar asked "can this result be written to?" on every frame, and the answer came from a
   fresh tree-sitter parse (`Parser::new` + `set_language` + a tree walk). It is now cached
   beside the table badges, in the branch that already recomputed only when the SQL changed.
3. Any change in zoom counted as a layout change, so `ColumnWidths::resolve` re-measured 30 rows
   of every column, and the schema roles were reclassified, for widths that are **world** units
   and cannot move when the camera does. `adopt` now reports rows, widths and scale separately.
4. `CanvasView::render` cloned every node on the page — including each agent node's whole message
   history — before culling. It culls first now, and prunes node state only when the document
   revision moved rather than by an O(states × nodes) scan every frame. Edge endpoints resolve
   through a per-frame index instead of two linear scans per edge.
5. Every visible string was re-shaped every frame, because gpui keys its line-layout and glyph
   caches on the exact font size. `peek_canvas::render_scale` snaps the content scale to a
   ladder; see `docs/canvas.md`, "Node zoom strategy".

Then `peek_canvas::lod` stops building bodies below zoom 0.32 at all — but only under
`--performance`. Without the flag the canvas draws every body at every zoom; the tier trades
away detail, so it is opt-in and the benchmark below turns it on explicitly.

`cargo test -p peek-ui --test frame_cost --release -- --ignored --nocapture` measures it: 24
result nodes of 2,000 × 8 cells, pinched from 100 % to 10 %. **39.5 ms a frame before, 30.8 after
items 1–4, 14.3 with LOD on top.** A pinch *into* the readable range went 9.3 → 7.5 ms.

**That benchmark cannot see item 5.** `TestPlatform` installs a `NoopTextSystem`, so a headless
frame shapes no text and rasterises no glyphs — the entire cost `render_scale` exists to remove
is absent from the harness, and disabling the snapping changes the headless number by nothing.
The evidence for it is the cache keys themselves (`RenderGlyphParams` and
`line_layout::CacheKey` both carry `font_size`, and `TextSystem::raster_bounds` is an unbounded
map that is never cleared), and confirming it needs the real app under `PEEK_FRAME_STATS=1`.

Deliberately left alone: `paint_dot_grid` emits one quad per dot but `grid_step` keeps them
≥ 12 px apart, which caps it near 11k quads for a full-screen pane; and the ~26 `on_action`
listeners, the HUD and the toolbar are rebuilt every frame, which is cheap next to the above.

## Known gaps and quirks

- **The picker's write paths are not driven end to end.** Saving a form, renaming a connection
  and removing one are unit-tested where the logic lives — `peek_config::workspaces` (the
  mutation API and its duplicate-name refusals) and `DocumentStore::rename` (moving the document
  and its sidecar, refusing an occupied target) — but no `#[gpui_kit::test]` clicks Save and
  watches a file move. `WorkspaceView::with_document` is `ReadOnly` by construction, and making
  it writable in a test would point `DocumentStore` at the real `~/peek`; doing this properly
  means threading a base directory through the view, which is worth doing when something else
  needs it. What the headless tests do cover is that the gate holds: Save and Duplicate change
  nothing under `ReadOnly`.
- Two kinds still show the placeholder body: `ResultInsertForm` and Activity.
- **The agent node talks to a real agent.** `a` or the toolbar places one; it streams an ACP
  session (Claude Code by default) or a local Ollama model, renders thoughts, plans, tool
  disclosures and permission prompts, and forks a conversation into a sibling node. A turn
  commits to the document as one undo step.
- **An agent can drive the canvas.** With `ai.mcp.enable` set, `peek-mcp` serves the 21 canvas
  tools on `127.0.0.1:<port>` and the drain answers each one against the live document. Verified
  end to end over HTTP: `get_db_schema` returns the connection's real DDL, `create_text_node`
  places a node, and an unknown id comes back as the reference's own error string.
- **Regions have a mutation API but no renderer.** `group_nodes`, `add_to_region` and
  `remove_region` write real regions with exclusive membership and undo; nothing draws them yet,
  so an agent that groups nodes leaves no visible trace. That is milestone order, not a bug.
- **The ACP session opens on the first prompt, not on mount.** The reference opens eagerly so the
  mode pill and the MCP warning are ready before the first question; here the pill appears after
  the first turn. Opening four sessions when a document loads seemed the worse trade.
- **No end-to-end test drives MCP over HTTP.** The two halves are covered — `peek-mcp`'s channel
  round-trip, and `CanvasView::run_tool` for all 21 tools — but nothing exercises the socket
  between them in CI. The manual probe above is the only check.
- **`contextKey` is not the reference's sha1.** It is an opaque dedupe token nothing compares
  across documents, so a transcript written here and reopened in the TypeScript app may insert
  one extra context message.
- **The Query node runs**, and its result shows its rows. `Query::Run` (`meta-enter`) executes
  against the connection opened at startup, places the result or a `query-error` node, and polls
  when live is on.
- **Query completions and diagnostics now see the schema.** `Database` fills
  `peek_lsp`'s `SchemaIndex` once a connection answers (129 tables on the local Plock database),
  so completions offer tables and columns and `diagnose` flags unknown ones. Before a connection
  answers it is empty, which is deliberate: the reference returns nothing rather than painting
  every table red while the schema loads.
- **Completions are filtered and ranked in `peek-lsp`, not by the menu.** Monaco did that job in
  the reference — the Rust crate there returns an unfiltered candidate set on purpose — and
  gpui-component's completion menu renders the provider's array verbatim, so `completion::ranking`
  does it now: drop what the typed prefix cannot mean, order by match class then kind then length,
  and set `filter_text` so the menu's highlight covers characters that actually matched. A `@`
  under the cursor answers with the node's canvas variables instead, which is the reference's
  second Monaco provider. Three limits remain, all in vendored gpui-component:
  `CompletionMenuItem::render` builds one `0..filter_text.len()` highlight run, so a match that
  does not start the label gets no highlight at all; there are no per-kind icons or colours (the
  reference's Tabler glyphs in `editor.css`); and `MAX_MENU_HEIGHT` is a hard-coded `px(240.)`,
  so the popover shows fewer rows as the camera zooms in even though its width now scales with it.
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
- No FPS overlay (`gpui-fps` is not a published crate for this gpui-pre version). `PEEK_FRAME_STATS=1`
  is the substitute: `canvas/frame_stats.rs` logs mean and p95 for render, prepaint and paint
  every 60 frames, with the visible and total node counts. See "Canvas frame cost" below.
- Fonts: the theme names "Monaspace Krypton" and relies on it being installed; bundling the OTFs
  via `cx.text_system().add_fonts` is deferred.
- A user keymap entry naming a command id not yet in the registry (e.g. `Page::New`) is logged and
  skipped at startup until that command lands.
