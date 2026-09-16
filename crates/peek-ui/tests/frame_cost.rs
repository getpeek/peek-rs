//! What one canvas frame costs on a page full of result nodes.
//!
//! Ignored by default; run it with
//! `cargo test -p peek-ui --test frame_cost --release -- --ignored --nocapture`.
//! Release matters: the debug build's allocator noise swamps the difference.
//!
//! **What this can and cannot see.** `TestPlatform` installs a `NoopTextSystem`, so a headless
//! frame builds elements, runs taffy and emits primitives, but shapes no text and rasterises no
//! glyphs. It therefore measures element construction and layout honestly, and says nothing at
//! all about `peek_canvas::render_scale` — whose whole purpose is to stop gpui re-shaping every
//! visible string when the font size moves. Measuring that needs the real app:
//! `PEEK_FRAME_STATS=1 cargo run`.

use std::time::Instant;

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Entity, Modifiers, PinchEvent, Pixels, Point, TestAppContext, TouchPhase,
    VisualTestContext, WindowHandle, point, px, size,
};
use peek_document::{CanvasDocument, Cell, Column, NodeId, ResultSet};
use peek_ui::{Launch, WorkspaceView};

const NODES: usize = 24;
const ROWS: usize = 2_000;
const COLUMNS: usize = 8;
/// How many of the eight columns hold JSON in the `json` fixture. Three is what a real events
/// or audit table looks like: a payload, some metadata, and a set of tags.
const JSON_COLUMNS: usize = 3;

fn document_json() -> String {
    let nodes: Vec<String> = (0..NODES)
        .map(|index| {
            let x = (index % 6) * 700;
            let y = (index / 6) * 520;
            format!(
                r#"{{"id":"query_bench{index:04}-result-0","type":"result",
                     "position":{{"x":{x},"y":{y}}},"width":600,"height":440,
                     "data":{{"query":"select * from users"}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"version":1,"activePageId":"page_bench001","pageOrder":["page_bench001"],
            "pages":{{"page_bench001":{{"id":"page_bench001","name":"Bench",
            "nodes":[{}],"edges":[],"viewport":{{"x":0,"y":0,"zoom":1}}}}}}}}"#,
        nodes.join(",")
    )
}

fn rows() -> ResultSet {
    let columns = (0..COLUMNS)
        .map(|c| Column::new(format!("column_{c}"), "VARCHAR"))
        .collect();
    let body = (0..ROWS)
        .map(|r| {
            (0..COLUMNS)
                .map(|c| Cell::Text(format!("value {r}:{c}")))
                .collect()
        })
        .collect();
    ResultSet::new(columns, body)
}

/// The same shape, but with three JSON columns.
///
/// The all-text fixture above cannot see the cost of a JSON cell at all — not the deep clone
/// `render_td` used to make of every visible value, and not the preview each one renders — so
/// every number it produced was silent about the column type that is most expensive to draw.
fn json_rows() -> ResultSet {
    let columns = (0..COLUMNS)
        .map(|c| {
            if c < JSON_COLUMNS {
                Column::new(format!("column_{c}"), "JSONB")
            } else {
                Column::new(format!("column_{c}"), "VARCHAR")
            }
        })
        .collect();
    let body = (0..ROWS)
        .map(|r| {
            (0..COLUMNS)
                .map(|c| {
                    if c < JSON_COLUMNS {
                        Cell::Json(payload(r, c))
                    } else {
                        Cell::Text(format!("value {r}:{c}"))
                    }
                })
                .collect()
        })
        .collect();
    ResultSet::new(columns, body)
}

/// A twelve-key object with a nested object and an array, which is the shape a jsonb column of
/// event metadata actually holds.
fn payload(row: usize, column: usize) -> serde_json::Value {
    serde_json::json!({
        "id": row,
        "column": column,
        "kind": "checkout.completed",
        "source": "web",
        "session": format!("sess_{row:08}"),
        "amount": format!("{row}.50"),
        "currency": "SEK",
        "live": row.is_multiple_of(2),
        "note": null,
        "actor": { "id": row, "email": format!("user{row}@example.com"), "role": "admin" },
        "tags": ["checkout", "payment", "web"],
        "trace": format!("{:064x}", row),
    })
}

fn open(cx: &mut TestAppContext, set: &ResultSet) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let mut config = peek_config::PeekConfig::default();
        config.theme = peek_config::ThemeId::Midday;
        // The whole point of this benchmark is the relief `--performance` buys, so it measures
        // the canvas that flag asks for, not the default one.
        let launch = Launch {
            performance: true,
            ..Launch::default()
        };
        peek_ui::init_with(&config, &launch, cx);
    });
    let json = document_json();
    let mut workspace = None;
    let handle = cx.open_window(size(px(1600.0), px(1000.0)), |window, cx| {
        let document = CanvasDocument::from_json(&json).unwrap();
        let view = cx.new(|cx| {
            let view = WorkspaceView::with_document("bench", document, window, cx);
            view.document(cx).update(cx, |document, _| {
                for index in 0..NODES {
                    document.set_result(
                        NodeId::from(format!("query_bench{index:04}-result-0").as_str()),
                        set.clone(),
                    );
                }
            });
            view
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (handle, workspace.unwrap())
}

struct Sweep<'a> {
    label: &'a str,
    set: ResultSet,
    factor: f32,
    steps: usize,
}

fn sweep(cx: &mut TestAppContext, sweep: Sweep<'_>) {
    let Sweep {
        label,
        set,
        factor,
        steps,
    } = sweep;
    let (handle, workspace) = open(cx, &set);
    cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
        .unwrap();

    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    let anchor: Point<Pixels> = point(px(800.0), px(500.0));
    let mut total = 0u128;
    for _ in 0..steps {
        let started = Instant::now();
        // A pinch both moves the camera and draws the frame it caused, which is the unit the
        // user actually feels.
        visual.simulate_event(PinchEvent {
            position: anchor,
            delta: factor,
            modifiers: Modifiers::default(),
            phase: TouchPhase::Moved,
        });
        total += started.elapsed().as_micros();
    }
    let zoom = cx.update(|cx| workspace.read(cx).camera(cx).zoom);
    #[allow(clippy::cast_precision_loss)]
    let mean = total as f64 / steps as f64 / 1000.0;
    println!("{label}: {mean:.2} ms/frame over {steps} frames, ending at zoom {zoom:.3}");
}

/// A pinch out from 100 % down past the detail threshold, one frame per step.
#[gpui_kit::test]
#[ignore = "benchmark"]
fn zooming_out_over_result_nodes(cx: &mut TestAppContext) {
    sweep(
        cx,
        Sweep {
            label: "text, pinch out 1.0 -> 0.1",
            set: rows(),
            factor: -0.03,
            steps: 80,
        },
    );
}

/// A pinch that stays where nodes are readable: the cost with no LOD relief at all.
#[gpui_kit::test]
#[ignore = "benchmark"]
fn zooming_within_the_readable_range(cx: &mut TestAppContext) {
    sweep(
        cx,
        Sweep {
            label: "text, pinch in 1.0 -> 4.0",
            set: rows(),
            factor: 0.02,
            steps: 70,
        },
    );
}

/// The readable range again, over JSON columns: the range where every visible cell parses,
/// previews and draws, and where a regression in the JSON path would actually show.
#[gpui_kit::test]
#[ignore = "benchmark"]
fn zooming_over_json_results(cx: &mut TestAppContext) {
    sweep(
        cx,
        Sweep {
            label: "json, pinch in 1.0 -> 4.0",
            set: json_rows(),
            factor: 0.02,
            steps: 70,
        },
    );
}
