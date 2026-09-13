use super::super::{Command, Group, WORKSPACE, WORKSPACE_NOT_TYPING, actions, always};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "CommandPalette::Open",
        title: "Command palette",
        label: None,
        group: Group::App,
        keywords: "",
        default_keys: &["meta-p", "meta-shift-p"],
        context: WORKSPACE,
        build: || Box::new(actions::command_palette::Open),
        available: always,
    },
    Command {
        id: "App::About",
        title: "About Peek",
        label: None,
        group: Group::App,
        keywords: "version info",
        default_keys: &[],
        context: WORKSPACE,
        build: || Box::new(actions::app::About),
        available: always,
    },
    Command {
        id: "ConnectionPicker::Open",
        title: "Change connection",
        label: None,
        // The connection is what this window is looking at, which puts it with Quit and About
        // rather than with a preference.
        group: Group::App,
        keywords: "open connection picker database switch workspace",
        default_keys: &["p"],
        // `WorkspaceView` handles this — switching a connection rebuilds the document, its
        // pages, the rows sidecar and the autosave, none of which the canvas can reach. Its
        // key context is `Workspace`, so a `Canvas`-scoped binding would never arrive, and the
        // picker has to open while the title bar holds focus too.
        context: WORKSPACE_NOT_TYPING,
        build: || Box::new(actions::connection_picker::Open),
        available: always,
    },
    Command {
        id: "App::Quit",
        title: "Quit Peek",
        label: None,
        group: Group::App,
        keywords: "exit",
        default_keys: &["meta-q"],
        context: WORKSPACE,
        build: || Box::new(actions::app::Quit),
        available: always,
    },
];
