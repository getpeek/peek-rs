//! `Export::Csv` / `Export::Json` end to end: the palette's action, the directory prompt, and
//! one file per selected result node written to the absolute path that came back.

use std::path::{Path, PathBuf};

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
use peek_canvas::Document;
use peek_document::geometry::{Point, Rect, Size};
use peek_document::{CanvasDocument, Cell, Column, NodeId, NodeType, ResultData, ResultSet};
use peek_ui::WorkspaceView;

/// A scratch directory that cleans up after itself. Hand-rolled rather than pulling
/// `tempfile` into the workspace for one test file.
#[derive(Debug)]
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("peek-export-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

fn open(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<WorkspaceView>) {
    cx.update(|cx| {
        let config = peek_config::PeekConfig::default();
        peek_ui::init(&config, cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1200.0), px(800.0)), |window, cx| {
        let view =
            cx.new(|cx| WorkspaceView::with_document("test", CanvasDocument::empty(), window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    (handle, workspace.unwrap())
}

/// A result node holding `sql` and one row per name, selected along with everything else this
/// was called for.
fn result_node(document: &mut Document, sql: &str, names: &[&str]) -> NodeId {
    let id = document.create_node(
        NodeType::Result,
        Rect::new(Point::default(), Size::new(600.0, 440.0)),
    );
    document.update_data::<ResultData>(&id, |data| data.query = sql.to_string());
    document.set_result(
        id.clone(),
        ResultSet::new(
            vec![Column::new("id", "INT4"), Column::new("name", "VARCHAR")],
            names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    vec![
                        Cell::Int(i64::try_from(index).unwrap()),
                        Cell::Text((*name).to_string()),
                    ]
                })
                .collect(),
        ),
    );
    id
}

/// Dispatches `action` through the window, answers the directory prompt with `directory`, and
/// lets the write finish.
fn export_into(
    (handle, cx): (WindowHandle<Root>, &mut TestAppContext),
    action: Box<dyn gpui_kit::Action>,
    directory: Option<PathBuf>,
) {
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(action, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories, "a directory, not a file");
        assert!(!options.files);
        assert!(!options.multiple);
        directory.map(|path| vec![path])
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn exporting_csv_writes_one_slugified_file_per_selected_result(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let directory = Scratch::new("csv");

    cx.update(|cx| {
        workspace.read(cx).document(cx).update(cx, |document, cx| {
            let users = result_node(document, "select * from users", &["ada", "grace"]);
            let orders = result_node(document, "select * from orders", &["first"]);
            document.select_only([users, orders]);
            cx.notify();
        });
    });

    export_into(
        (handle, cx),
        Box::new(peek_ui::commands::actions::export::Csv),
        Some(directory.path().to_path_buf()),
    );

    let users = std::fs::read_to_string(directory.path().join("select_from_users.csv")).unwrap();
    assert_eq!(users, "id;name\n\"0\";\"ada\"\n\"1\";\"grace\"");
    let orders = std::fs::read_to_string(directory.path().join("select_from_orders.csv")).unwrap();
    assert_eq!(orders, "id;name\n\"0\";\"first\"");
}

#[gpui_kit::test]
fn exporting_json_writes_a_compact_array_of_objects(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let directory = Scratch::new("json");

    cx.update(|cx| {
        workspace.read(cx).document(cx).update(cx, |document, cx| {
            let users = result_node(document, "select * from users", &["ada"]);
            document.select_only([users]);
            cx.notify();
        });
    });

    export_into(
        (handle, cx),
        Box::new(peek_ui::commands::actions::export::Json),
        Some(directory.path().to_path_buf()),
    );

    let written = std::fs::read_to_string(directory.path().join("select_from_users.json")).unwrap();
    assert_eq!(written, r#"[{"id":0,"name":"ada"}]"#);
}

/// Cancelling the picker must not write anything, which is the one branch a user hits by
/// accident.
#[gpui_kit::test]
fn cancelling_the_directory_prompt_writes_nothing(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);
    let directory = Scratch::new("cancelled");

    cx.update(|cx| {
        workspace.read(cx).document(cx).update(cx, |document, cx| {
            let users = result_node(document, "select * from users", &["ada"]);
            document.select_only([users]);
            cx.notify();
        });
    });

    export_into(
        (handle, cx),
        Box::new(peek_ui::commands::actions::export::Csv),
        None,
    );

    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

/// The palette dispatches through the canvas focus handle, so a selection of nothing must not
/// leave a prompt queued for the next command to answer.
#[gpui_kit::test]
fn exporting_with_no_result_selected_never_asks_for_a_directory(cx: &mut TestAppContext) {
    let (handle, workspace) = open(cx);

    cx.update(|cx| {
        workspace.read(cx).document(cx).update(cx, |document, cx| {
            document.create_node(
                NodeType::Query,
                Rect::new(Point::default(), Size::new(400.0, 300.0)),
            );
            document.select_all();
            cx.notify();
        });
    });

    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(peek_ui::commands::actions::export::Csv), cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert!(!cx.did_prompt_for_paths());
}
