//! Action types, one module per Peek group so short names never collide. `actions!` names
//! each one `"Group::Variant"`, byte-identical to the ids in `settings.json`'s keymap.

use gpui_kit::actions;

pub mod tool {
    use super::actions;
    actions!(
        Tool,
        [Select, LassoSelect, Query, Agent, Text, Variable, Draw]
    );
}

pub mod edit {
    use super::actions;
    actions!(Edit, [Cut, Paste, SelectAll, DeleteSelection]);
    /// `Copy` shadows the marker trait inside the module that declares it, so it gets a module
    /// of its own. The id still has to be exactly `Edit::Copy`: that is the name the reference's
    /// `settings.json` keymap uses, and a user rebinding it must reach this action.
    pub mod copy {
        use super::super::actions;
        actions!(Edit, [Copy]);
    }
}

pub mod history {
    use super::actions;
    actions!(History, [Undo, Redo]);
}

pub mod zoom {
    use super::actions;
    actions!(Zoom, [In, Out, Reset, FitView, FitSelection]);
}

pub mod page {
    use super::actions;
    actions!(
        Page,
        [
            New,
            Close,
            Previous,
            Next,
            GoToNode,
            Search,
            OpenPicker,
            SelectNodeLeft,
            SelectNodeRight,
            SelectNodeUp,
            SelectNodeDown
        ]
    );
}

/// Query-node commands. `Format` is dispatched while the SQL editor holds focus, which is why
/// the reference binds it with a modifier rather than a bare key. `Focus` is the opposite: it
/// fires from the canvas and hands focus *to* the editor.
pub mod query {
    use super::actions;
    actions!(Query, [Format, Focus, Run]);
}

pub mod view {
    use super::actions;
    actions!(View, [ToggleUi, ToggleCameraLock, ShowRunningQueries]);
}

pub mod command_palette {
    use super::actions;
    actions!(CommandPalette, [Open]);
}

pub mod app {
    use super::actions;
    actions!(App, [Quit]);
}

pub mod theme {
    use super::actions;
    actions!(Theme, [Open]);
}
