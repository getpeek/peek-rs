use super::super::{CANVAS_NOT_TYPING, Command, Group, WORKSPACE, actions, always};
use peek_canvas::Scope;

/// The reference names both of these by what pressing them *does*, not by what they control, so
/// the label has to follow the state. `Scope` already carried `chrome_hidden` and
/// `camera_locked`; these are their first readers.
fn interface_label(scope: &Scope) -> &'static str {
    if scope.chrome_hidden {
        "Show UI"
    } else {
        "Hide UI"
    }
}

/// The schema page is built from the live database, so there has to be one.
fn connected(scope: &Scope) -> bool {
    scope.connected
}

fn camera_lock_label(scope: &Scope) -> &'static str {
    if scope.camera_locked {
        "Unlock camera"
    } else {
        "Lock camera"
    }
}

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "View::ToggleCameraLock",
        title: "Lock or unlock the camera",
        label: Some(camera_lock_label),
        group: Group::View,
        keywords: "freeze pan zoom",
        default_keys: &["meta-shift-l"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::view::ToggleCameraLock),
        available: always,
    },
    Command {
        id: "View::ToggleUi",
        title: "Show or hide the interface",
        label: Some(interface_label),
        group: Group::View,
        keywords: "chrome focus mode zen presentation",
        default_keys: &["meta-."],
        context: WORKSPACE,
        build: || Box::new(actions::view::ToggleUi),
        available: always,
    },
    Command {
        id: "Theme::Open",
        title: "Change theme",
        label: None,
        group: Group::View,
        keywords: "colors appearance dark light pine midnight midday terminal paper blueprint",
        default_keys: &[],
        context: WORKSPACE,
        build: || Box::new(actions::theme::Open),
        available: always,
    },
    Command {
        id: "View::Organize",
        title: "Organize canvas",
        label: None,
        group: Group::View,
        keywords: "layout arrange auto force directed graph fit",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::view::Organize),
        available: always,
    },
    Command {
        id: "View::Schema",
        title: "View schema",
        label: None,
        group: Group::View,
        keywords: "database tables columns foreign keys diagram",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::view::Schema),
        available: connected,
    },
];
