//! Action types, one module per Peek group so short names never collide. `actions!` names
//! each one `"Group::Variant"`, byte-identical to the ids in `settings.json`'s keymap.
//!
//! Every action this build knows is declared here, including ones whose registry entry and
//! handler land later: declaring them together keeps one file the single place a name is
//! coined, and an action with no entry is simply never bound.

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
    actions!(
        Zoom,
        [In, Out, Reset, FitView, FitSelection, FitSelectionAndLock]
    );
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
            SelectNodeDown,
            SelectPreviousQuery,
            SelectNextQuery
        ]
    );

    /// Switching to one named page. Data-carrying, so unlike everything else here it has no
    /// registry entry and no default key: the palette generates one row per page, and there is
    /// nothing stable for a user to bind. `no_json` keeps it out of the keymap schema for the
    /// same reason.
    #[derive(Clone, PartialEq, Eq, Debug, gpui_kit::Action)]
    #[action(namespace = Page, no_json)]
    pub struct GoTo {
        pub page: peek_document::PageId,
    }
}

/// Query-node commands. `Format` is dispatched while the SQL editor holds focus, which is why
/// the reference binds it with a modifier rather than a bare key. `Focus` is the opposite: it
/// fires from the canvas and hands focus *to* the editor.
pub mod query {
    use super::actions;
    actions!(Query, [Format, Focus, Run, RerunAll, RerunSelected]);
}

/// Result-node commands.
pub mod result {
    use super::actions;
    actions!(
        Result,
        [
            Pivot,
            Chart,
            CopyValue,
            CopyAsJson,
            CopyAsCsv,
            CopyAsSql,
            ExportAsJson,
            ExportAsCsv,
            ExportAsSql,
            UseAsVariable,
            DeleteRows
        ]
    );
}

/// Agent-node commands. `CycleMode` and `Stop` fire while the composer holds focus, which is
/// why they sit on the node's own context rather than the canvas'.
pub mod agent {
    use super::actions;
    actions!(Agent, [Fork, CycleMode, Stop]);
}

pub mod view {
    use super::actions;
    actions!(View, [ToggleUi, ToggleCameraLock, Organize, Schema]);
}

/// Region commands. `GroupSelection` and `UngroupSelection` act on the selection;
/// `OpenPicker` raises the regions menu in the zoom cluster.
pub mod region {
    use super::actions;
    actions!(Region, [GroupSelection, UngroupSelection, OpenPicker]);
}

pub mod export {
    use super::actions;
    actions!(Export, [Csv, Json]);
}

pub mod settings {
    use super::actions;
    actions!(
        Settings,
        [ToggleCommandPaletteButton, TogglePageDisplay, ToggleRegions]
    );
}

pub mod help {
    use super::actions;
    actions!(Help, [Keymap]);
}

pub mod command_palette {
    use super::actions;
    actions!(CommandPalette, [Open]);
}

pub mod connection_picker {
    use super::actions;
    actions!(ConnectionPicker, [Open]);
}

pub mod app {
    use super::actions;
    actions!(App, [Quit, About]);
}

pub mod theme {
    use super::actions;
    actions!(Theme, [Open]);
}
