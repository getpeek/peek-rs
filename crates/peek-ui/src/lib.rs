//! gpui-kit views for Peek: the window, the canvas and its node shells, and the command
//! registry that keyboard shortcuts, the palette and buttons all dispatch through.

use gpui_kit::component::Root;
use gpui_kit::prelude::*;
use gpui_kit::{Bounds, WindowBounds, WindowOptions, px, size};
use peek_config::{PeekConfig, PersistenceMode};

mod about;
mod assets;
mod autosave;
mod canvas;
pub mod commands;
mod database;
mod execution;
mod fuzzy;
mod keymap_help;
mod mcp;
mod node;
mod settings;
mod theme_picker;
mod title_bar;
mod workspace;

pub use workspace::WorkspaceView;

/// Readers for state the UI keeps in gpui globals, where an integration test — which links this
/// crate as an outside user — otherwise cannot see it. Nothing in the app calls these.
pub mod test_support {
    use crate::settings::Settings;

    /// The connections configured under `workspace`, in file order.
    #[must_use]
    pub fn connection_names(cx: &gpui_kit::App, workspace: &str) -> Vec<String> {
        Settings::get(cx)
            .workspace(workspace)
            .map(|workspace| {
                workspace
                    .connections
                    .iter()
                    .map(|connection| connection.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Command-line launch options.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub workspace: Option<String>,
    pub connection: Option<String>,
    /// Whether this run may write to `~/peek`. Opt-in while the TypeScript app still owns the
    /// same files; it autosaves on its own three-second debounce.
    pub persistence: PersistenceMode,
    /// Whether the canvas may trade detail for frame time. Opt-in: without it every node
    /// builds its real body at every zoom, which is what the level of detail in
    /// [`peek_canvas::lod`] otherwise stops doing once a card is too small to read.
    pub performance: bool,
    /// Whether the zoom cluster carries a frame-rate readout. Opt-in because it is a debugging
    /// instrument, and because it is the one thing here that adds work to the frame loop: a
    /// settle timer, so the reading can fall back to `idle` once the canvas stops drawing.
    pub fps: bool,
}

impl Launch {
    /// Parses
    /// `--workspace <name> --connection <name> [--write | --read-only] [--performance] [--fps]`.
    #[must_use]
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut launch = Self::default();
        let mut args = args.into_iter().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--workspace" => launch.workspace = args.next(),
                "--connection" => launch.connection = args.next(),
                "--write" => launch.persistence = PersistenceMode::ReadWrite,
                "--read-only" => launch.persistence = PersistenceMode::ReadOnly,
                "--performance" => launch.performance = true,
                "--fps" => launch.fps = true,
                _ => log::warn!("peek: ignoring unknown argument {arg:?}"),
            }
        }
        launch
    }
}

/// Installs everything a window needs before it opens: gpui-kit, the theme globals, the
/// keymap and the app-level actions, as if launched with no arguments. Tests call this
/// instead of [`run`].
pub fn init(config: &PeekConfig, cx: &mut gpui_kit::App) {
    init_with(config, &Launch::default(), cx);
}

/// [`init`], for a test that needs the launch options a flag would have set.
pub fn init_with(config: &PeekConfig, launch: &Launch, cx: &mut gpui_kit::App) {
    gpui_kit::init(cx);
    settings::Settings::init(config.clone(), launch, cx);
    register_sql_grammar();
    node::query::language::SqlLanguage::init(cx);
    database::Database::init(cx);
    node::agent::backend::Agents::init(config, cx);
    peek_theme::ThemeService::init(config.theme, cx);
    commands::keymap::bind(&config.keymap, cx);
    cx.on_action(|_: &commands::actions::app::Quit, cx| cx.quit());
}

/// Overrides the bundled `sql` grammar with one whose captures match the roles a Peek theme
/// colours. Registering under the same name replaces the built-in entry, so every
/// `EditorState::language("sql")` resolves to this one.
fn register_sql_grammar() {
    use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

    LanguageRegistry::singleton().register(
        "sql",
        &GrammarConfig::new(
            "sql",
            peek_lsp::sql_language(),
            Vec::new(),
            &peek_lsp::sql_highlights(),
            "",
            "",
        ),
    );
}

/// Starts the application. Blocks until the last window closes.
///
/// # Panics
/// If the main window cannot be opened; there is nothing to show without it.
pub fn run(launch: Launch) {
    let app = gpui_kit::application().with_assets(assets::Assets);
    app.run(move |cx| {
        // Shared with the workspace view, which keeps it to reconnect on a connection switch.
        let config = std::rc::Rc::new(PeekConfig::get_or_default());
        if let Err(error) = PeekConfig::ensure_initialized_on_disk(launch.persistence) {
            log::warn!("peek: {error}");
        }
        init_with(&config, &launch, cx);

        let bounds = Bounds::centered(None, size(px(1280.0), px(840.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // The bottom chrome is two panels, one pinned left and one centred, at the
            // reference's own sizes; below about 750 px the centred one runs into the other.
            window_min_size: Some(size(px(760.0), px(480.0))),
            ..title_bar::window_options()
        };
        cx.open_window(options, |window, cx| {
            let workspace = cx.new(|cx| WorkspaceView::new(config.clone(), &launch, window, cx));
            cx.new(|cx| Root::new(workspace, window, cx))
        })
        .expect("the main window opens");
        cx.activate(true);
    });
}

#[cfg(test)]
mod launch_tests {
    use super::Launch;
    use peek_config::PersistenceMode;

    /// `from_args` skips the first argument, which is the binary's own path.
    fn parse(args: &[&str]) -> Launch {
        let mut all = vec!["peek".to_string()];
        all.extend(args.iter().map(|arg| (*arg).to_string()));
        Launch::from_args(all)
    }

    #[test]
    fn the_defaults_are_read_only_with_no_flags() {
        let launch = parse(&[]);
        assert_eq!(launch.persistence, PersistenceMode::ReadOnly);
        assert!(!launch.performance);
        assert!(!launch.fps);
        assert_eq!(launch.workspace, None);
    }

    #[test]
    fn the_named_workspace_and_connection_are_taken_as_pairs() {
        let launch = parse(&["--workspace", "Plock", "--connection", "local"]);
        assert_eq!(launch.workspace.as_deref(), Some("Plock"));
        assert_eq!(launch.connection.as_deref(), Some("local"));
    }

    /// The three switches are independent: `--fps` must not imply the level-of-detail trade,
    /// which would change what the readout is measuring.
    #[test]
    fn the_switches_do_not_imply_each_other() {
        let launch = parse(&["--fps"]);
        assert!(launch.fps);
        assert!(!launch.performance);
        assert_eq!(launch.persistence, PersistenceMode::ReadOnly);

        let launch = parse(&["--performance"]);
        assert!(launch.performance);
        assert!(!launch.fps);

        let launch = parse(&["--write", "--fps", "--performance"]);
        assert!(launch.fps);
        assert!(launch.performance);
        assert_eq!(launch.persistence, PersistenceMode::ReadWrite);
    }

    /// An unknown argument is logged and skipped rather than taken as a value, so one typo does
    /// not swallow the flag after it.
    #[test]
    fn an_unknown_argument_does_not_swallow_the_next_one() {
        let launch = parse(&["--nonsense", "--fps"]);
        assert!(launch.fps);
    }
}
