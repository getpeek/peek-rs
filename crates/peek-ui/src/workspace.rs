//! The window's content: title bar, canvas, and the window-level commands (palette, chrome
//! visibility). Owns the loaded document and the canvas focus handle that commands from
//! non-canvas surfaces (palette, buttons) dispatch through.

use std::rc::Rc;

use gpui_kit::component::command::{Command as CommandPalette, CommandItem, CommandState};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::{ActiveTheme, Root, StyledExt, WindowExt};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, CursorStyle, Entity, FocusHandle, SharedString, Subscription, Window, div,
};
use peek_canvas::{Camera, Document};
use peek_config::{PeekConfig, PersistenceMode};

use crate::database::Database;
use peek_document::{
    CanvasDocument, DocumentFile, DocumentStore, ResultSet, ResultSidecar, ResultsFile,
};

use crate::Launch;
use crate::autosave::Autosave;
use crate::canvas::CanvasView;
use crate::commands::{self, COMMANDS, actions};
use crate::title_bar::PeekTitleBar;
use crate::title_bar::connection::{self, Choice, ConnectionPicker};
use crate::title_bar::pages::PageTabs;

pub struct WorkspaceView {
    title: SharedString,
    /// Which document is open, so the picker can mark it current and skip a switch to itself.
    /// Empty for [`WorkspaceView::with_document`], which is handed a document rather than a name.
    workspace: String,
    connection: String,
    /// What the picker offers, snapshotted from `settings.json` at startup. The reference reads
    /// its config once too, and rendering must not touch the disk.
    connections: Rc<[Choice]>,
    /// The same snapshot, unflattened: switching needs the url and ssh tunnel behind a choice,
    /// which `Choice` deliberately does not carry into the title bar.
    config: Rc<PeekConfig>,
    canvas: Entity<CanvasView>,
    canvas_focus: FocusHandle,
    palette: Entity<CommandState>,
    theme_picker: Entity<CommandState>,
    page_tabs: Entity<PageTabs>,
    persistence: PersistenceMode,
    ui_visible: bool,
    /// Repaints when the connection resolves. `Database` is a global mutated from a spawned
    /// task, so without this the pill, `Scope::connected` and the Run button all keep rendering
    /// whatever was true at the last unrelated repaint.
    _database: Subscription,
    /// Only present when this run may write; its absence is what makes read-only safe.
    autosave: Option<Entity<Autosave>>,
}

impl std::fmt::Debug for WorkspaceView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceView")
            .field("title", &self.title)
            .field("ui_visible", &self.ui_visible)
            .finish_non_exhaustive()
    }
}

impl WorkspaceView {
    pub fn new(
        config: Rc<PeekConfig>,
        launch: &Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (workspace, connection) = pick_connection(&config, launch);
        let title = format!("{workspace} / {connection}");
        let loaded = load_document(&workspace, &connection, launch.persistence);

        let mut view = Self::with_document(title, loaded.document, window, cx);
        view.persistence = launch.persistence;
        view.workspace = workspace;
        view.connection = connection;
        view.connections = connection::choices(&config);
        view.config = config;
        view.adopt_results(loaded.results, cx);
        connect_to(&view.config, &view.workspace, &view.connection, cx);
        view.autosave = Self::autosave_for(
            &view.document(cx),
            (loaded.file, loaded.results_file),
            launch.persistence,
            cx,
        );
        view
    }

    /// An autosave over `file`, or `None` when this run may not write — its absence is what makes
    /// read-only safe, so the mode is checked here and nowhere else.
    fn autosave_for(
        document: &Entity<Document>,
        files: (Option<DocumentFile>, Option<ResultsFile>),
        persistence: PersistenceMode,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Autosave>> {
        let (file, results_file) = files;
        let file = file.filter(|_| persistence.can_write())?;
        let results_file = results_file.filter(|_| persistence.can_write());
        let document = document.clone();
        Some(cx.new(|cx| Autosave::new(document, (file, results_file), cx)))
    }

    /// A workspace over an already loaded document; the constructor tests use.
    pub fn with_document(
        title: impl Into<SharedString>,
        document: CanvasDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = title.into();
        let document = cx.new(|_| Document::load(document));

        let canvas_focus = cx.focus_handle();
        let canvas = cx.new(|cx| CanvasView::new(document, canvas_focus.clone(), window, cx));
        let page_tabs = cx.new(|cx| PageTabs::new(canvas.clone(), canvas_focus.clone(), cx));
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let theme_picker = cx.new(|cx| CommandState::new(window, cx));
        window.focus(&canvas_focus, cx);

        Self {
            title,
            workspace: String::new(),
            connection: String::new(),
            connections: Rc::from([]),
            config: Rc::new(PeekConfig::default()),
            canvas,
            canvas_focus,
            page_tabs,
            palette,
            theme_picker,
            persistence: PersistenceMode::ReadOnly,
            ui_visible: true,
            _database: cx.observe_global::<Database>(|_, cx| cx.notify()),
            autosave: None,
        }
    }

    /// Hands the rows loaded from the sidecar to the session document.
    fn adopt_results(&self, results: ResultSidecar, cx: &mut Context<Self>) {
        self.document(cx)
            .update(cx, |document, _| document.adopt_results(results));
    }

    /// The workspace and connection whose document is open.
    #[must_use]
    pub fn open_connection(&self) -> (&str, &str) {
        (&self.workspace, &self.connection)
    }

    /// Opens another connection's document in this window, as the reference's picker does: the
    /// canvas, its pages and its autosave are all rebuilt around the new document.
    ///
    /// The outgoing document is flushed first — autosave is a three-second debounce, so a switch
    /// within three seconds of an edit would otherwise drop it.
    pub fn switch_connection(
        &mut self,
        workspace: String,
        connection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if workspace == self.workspace && connection == self.connection {
            return;
        }
        if let Some(autosave) = self.autosave.take() {
            autosave.update(cx, Autosave::flush);
        }

        let loaded = load_document(&workspace, &connection, self.persistence);
        let results = loaded.results;
        let files = (loaded.file, loaded.results_file);
        let document = cx.new(|document_cx| {
            let mut session = Document::load(loaded.document);
            let _ = document_cx;
            session.adopt_results(results);
            session
        });
        // The focus handle outlives the canvas: every chrome button dispatches through it, and a
        // fresh one would leave the title bar firing actions at a dead handle.
        let canvas =
            cx.new(|cx| CanvasView::new(document.clone(), self.canvas_focus.clone(), window, cx));
        let visible = self.ui_visible;
        canvas.update(cx, |canvas, cx| canvas.set_chrome_visible(visible, cx));

        self.page_tabs = cx.new(|cx| PageTabs::new(canvas.clone(), self.canvas_focus.clone(), cx));
        self.canvas = canvas;
        self.autosave = Self::autosave_for(&document, files, self.persistence, cx);
        self.title = SharedString::from(format!("{workspace} / {connection}"));
        self.workspace = workspace;
        self.connection = connection;
        // The database follows the document. Without this the previous connection stays open and
        // its schema keeps answering completions for a database that is no longer on screen.
        let config = Rc::clone(&self.config);
        connect_to(&config, &self.workspace, &self.connection, cx);

        window.focus(&self.canvas_focus, cx);
        cx.notify();
    }

    /// The camera the canvas is showing or flying towards.
    #[must_use]
    pub fn camera(&self, cx: &App) -> Camera {
        self.canvas.read(cx).camera()
    }

    #[must_use]
    pub fn camera_target(&self, cx: &App) -> Camera {
        self.canvas.read(cx).camera_target()
    }

    #[must_use]
    /// The canvas' focus handle, so a test can tell whether focus sits on the canvas or has
    /// been handed to a node's editor.
    pub fn canvas_focus(&self) -> &FocusHandle {
        &self.canvas_focus
    }

    pub fn document(&self, cx: &App) -> Entity<Document> {
        self.canvas.read(cx).document().clone()
    }

    /// The table behind a result node, for tests.
    #[cfg(test)]
    pub(crate) fn result_table(
        &self,
        node: &peek_document::NodeId,
        cx: &App,
    ) -> Option<
        gpui_kit::Entity<
            gpui_kit::component::table::TableState<crate::node::result::ResultDelegate>,
        >,
    > {
        self.canvas.read(cx).result_table(node, cx)
    }

    /// The entity behind a result node, for tests.
    #[cfg(test)]
    pub(crate) fn result_inner(
        &self,
        node: &peek_document::NodeId,
        cx: &App,
    ) -> Option<gpui_kit::Entity<crate::node::result::ResultTable>> {
        self.canvas.read(cx).result_inner(node)
    }

    #[must_use]
    pub fn is_camera_locked(&self, cx: &App) -> bool {
        self.canvas.read(cx).is_camera_locked()
    }

    /// The cursor the canvas is asking for, which is the only observable trace of what the
    /// pointer is over: gpui exposes no window cursor to tests.
    #[must_use]
    pub fn cursor(&self, cx: &App) -> CursorStyle {
        self.canvas.read(cx).cursor()
    }

    fn toggle_ui(&mut self, _: &actions::view::ToggleUi, _: &mut Window, cx: &mut Context<Self>) {
        self.ui_visible = !self.ui_visible;
        let visible = self.ui_visible;
        self.canvas
            .update(cx, |canvas, cx| canvas.set_chrome_visible(visible, cx));
        cx.notify();
    }

    fn open_theme_picker(
        &mut self,
        _: &actions::theme::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::theme_picker::open(&self.theme_picker, self.persistence, window, cx);
    }

    fn open_palette(
        &mut self,
        _: &actions::command_palette::Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scope = self.canvas.read(cx).scope(cx);
        log::debug!("peek: opening palette with scope {scope:?}");
        let available: Vec<&'static commands::Command> = COMMANDS
            .iter()
            .filter(|command| command.id != "CommandPalette::Open")
            .filter(|command| (command.available)(&scope))
            .collect();
        let state = self.palette.clone();
        let focus = self.canvas_focus.clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let dialog = bare(dialog);
            let available = available.clone();
            let focus = focus.clone();
            let state = state.clone();
            dialog.overlay_closable(true).content(move |content, _, _| {
                let focus = focus.clone();
                let items = available.iter().map(|command| {
                    CommandItem::new()
                        .label(command.title)
                        .keywords(command.keywords.split_whitespace().map(str::to_owned))
                });
                let confirmed = available.clone();
                content.child(
                    CommandPalette::new(&state)
                        .items(items)
                        .on_confirm(move |path, window, cx| {
                            window.close_dialog(cx);
                            if let Some(command) = confirmed.get(path.row) {
                                focus.dispatch_action(&*(command.build)(), window, cx);
                            }
                        })
                        .on_cancel(WindowExt::close_dialog),
                )
            })
        });
        self.palette
            .update(cx, |palette, cx| palette.focus(window, cx));
    }
}

/// Opens the database behind the connection just loaded, if `settings.json` describes one.
///
/// Connecting is fire-and-forget: the canvas is already usable, and `Scope::connected` flips the
/// Run button on when the connection answers.
fn connect_to(config: &PeekConfig, workspace: &str, connection: &str, cx: &mut App) {
    let found = config
        .workspaces
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(workspace))
        .and_then(|found| {
            found
                .connections
                .iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(connection))
        });
    let Some(found) = found else {
        log::info!("peek: no connection named {workspace}/{connection} in settings.json");
        return;
    };
    Database::connect(found, cx);
}

fn pick_connection(config: &PeekConfig, launch: &Launch) -> (String, String) {
    let workspace = launch
        .workspace
        .clone()
        .or_else(|| {
            config
                .workspaces
                .first()
                .map(|workspace| workspace.name.clone())
        })
        .unwrap_or_else(|| "default".to_string());
    let connection = launch.connection.clone().or_else(|| {
        config
            .workspaces
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&workspace))
            .and_then(|found| found.connections.first())
            .map(|connection| connection.name.clone())
    });
    (
        workspace,
        connection.unwrap_or_else(|| "default".to_string()),
    )
}

/// Everything a connection's files yield: the document, its rows, and the handles autosave
/// writes back through.
struct Loaded {
    document: CanvasDocument,
    results: ResultSidecar,
    file: Option<DocumentFile>,
    results_file: Option<ResultsFile>,
}

/// Loads a connection's document and its rows sidecar.
fn load_document(workspace: &str, connection: &str, mode: PersistenceMode) -> Loaded {
    let store = match DocumentStore::new(mode) {
        Ok(store) => store,
        Err(error) => {
            log::error!("peek: could not open the document store: {error}");
            return Loaded {
                document: CanvasDocument::empty(),
                results: ResultSidecar::default(),
                file: None,
                results_file: None,
            };
        }
    };

    let mut file = match store.open(workspace, connection) {
        Ok(file) => file,
        Err(error) => {
            log::error!("peek: could not open {workspace}/{connection}: {error}");
            return Loaded {
                document: CanvasDocument::empty(),
                results: ResultSidecar::default(),
                file: None,
                results_file: None,
            };
        }
    };
    let mut document = match file.load() {
        Ok(Some(document)) => document,
        Ok(None) => {
            log::info!("peek: no document for {workspace}/{connection}, starting empty");
            CanvasDocument::empty()
        }
        Err(error) => {
            log::error!("peek: could not load {workspace}/{connection}: {error}");
            CanvasDocument::empty()
        }
    };

    // Rows inlined by a pre-sidecar version of the app are lifted first, so the sidecar wins
    // where both have rows for a result — the merge order `useLoadDocument.ts` uses.
    let mut results = lift_legacy_rows(&mut document);
    let mut results_file = store.open_results(workspace, connection).ok();
    if let Some(handle) = results_file.as_mut() {
        match handle.load() {
            Ok(sidecar) => {
                for id in sidecar.node_ids().cloned().collect::<Vec<_>>() {
                    if let Some(rows) = sidecar.get(&id) {
                        results.insert(id, rows.clone());
                    }
                }
            }
            Err(error) => log::error!("peek: could not load results for {connection}: {error}"),
        }
    }
    log::info!(
        "peek: {} result sets loaded for {workspace}/{connection}",
        results.len()
    );

    for note in peek_document::normalize(&mut document) {
        log::warn!("peek: {note}");
    }
    Loaded {
        document,
        results,
        file: Some(file),
        results_file,
    }
}

/// Moves any inline `data.data` rows a pre-sidecar document still carries into a sidecar, and
/// clears the field so the next save drops it. `ResultData::legacy_rows` is read-only for
/// exactly this reason: it is never written back.
fn lift_legacy_rows(document: &mut CanvasDocument) -> ResultSidecar {
    let mut lifted = ResultSidecar::default();
    for page in document.pages.values_mut() {
        for node in &mut page.nodes {
            let peek_document::NodeKind::Result(data) = &mut node.kind else {
                continue;
            };
            let Some(rows) = data.legacy_rows.take() else {
                continue;
            };
            let set = ResultSet::from_sidecar_rows(&rows);
            if !set.is_empty() {
                log::info!(
                    "peek: lifted {} inline rows from {}",
                    set.row_count(),
                    node.id
                );
                lifted.insert(node.id.clone(), set);
            }
        }
    }
    lifted
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .key_context(commands::WORKSPACE)
            .on_action(cx.listener(Self::toggle_ui))
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::open_theme_picker))
            // The canvas fills the window and the bar floats over it, as the reference does.
            // Order matters twice over: the bar must paint above the canvas, and its hitbox
            // must be inserted *after* the canvas' so `occlude()` can block it — without that
            // a click on a page tab also reaches the canvas and starts a marquee.
            .child(div().absolute().inset_0().child(self.canvas.clone()))
            .when(self.ui_visible, |this| {
                let picker = ConnectionPicker::new(
                    cx.entity(),
                    (
                        SharedString::from(self.workspace.clone()),
                        SharedString::from(self.connection.clone()),
                    ),
                    self.connections.clone(),
                );
                this.child(div().absolute().top_0().left_0().right_0().occlude().child(
                    PeekTitleBar::new(picker, self.page_tabs.clone(), self.canvas_focus.clone()),
                ))
            })
            // `Root` owns dialogs, sheets and notifications but leaves mounting them to the
            // window's first view, so the palette dialog is rendered here.
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

/// The command palette and theme picker draw their own themed surface, so the dialog must not
/// draw a second one behind it: `Dialog`'s popup fills `tokens.background`, outlines it and
/// insets the content by 16 px, which reads as a mismatched frame around the palette. Padding
/// is taken from the dialog's own `StyleRefinement`, so zeroing it removes the inset too.
pub(crate) fn bare(dialog: Dialog) -> Dialog {
    dialog
        .p_0()
        .bg(gpui_kit::transparent_black())
        .border_color(gpui_kit::transparent_black())
}
