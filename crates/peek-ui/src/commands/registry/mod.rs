//! The registry, one file per group. Splitting it is not cosmetic: a single array is a merge
//! conflict every time two features land commands at once, and the groups are the natural seam
//! because a command's group is already the thing that decides where its handler lives.
//!
//! Adding a command means adding an entry to its group's file. Nothing else in `commands`
//! needs to know: [`GROUPS`] is walked by `commands::all`, which is what the palette, the
//! keymap, the toolbar and the registry tests all read.

pub(super) mod agent;
pub(super) mod app;
pub(super) mod edit;
pub(super) mod export;
pub(super) mod help;
pub(super) mod history;
pub(super) mod page;
pub(super) mod query;
pub(super) mod result;
pub(super) mod settings;
pub(super) mod tool;
pub(super) mod view;
pub(super) mod zoom;

use super::Command;

pub(super) static GROUPS: &[&[Command]] = &[
    zoom::ENTRIES,
    edit::ENTRIES,
    tool::ENTRIES,
    history::ENTRIES,
    query::ENTRIES,
    result::ENTRIES,
    page::ENTRIES,
    view::ENTRIES,
    agent::ENTRIES,
    export::ENTRIES,
    settings::ENTRIES,
    help::ENTRIES,
    app::ENTRIES,
];
