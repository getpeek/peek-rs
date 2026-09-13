//! The workspace and connection picker: the panel under the title bar's pill.
//!
//! Ported from `~/labs/peek/src/Connection/` — `WorkspacePopover.tsx` for the shell,
//! `WorkspaceList.tsx` for the search and the cursor, `ConnectionItem.tsx` for a row.
//!
//! Three things about it are ours rather than the reference's:
//!
//! - **No `backdrop-filter`**, as everywhere else in this crate. gpui's only blur is
//!   `BoxShadow::blur_radius`, so the panel is opaque rather than frosted.
//! - **A scrim, not a capture-phase click-away.** The reference needs `useClickAwayCapture`
//!   because React Flow stops bubble-phase propagation; an occluding full-window sibling does
//!   the same job here, and `canvas/jump.rs` already establishes it.
//! - **The search box is a real focus owner**, which is what makes bare `p` safe: every
//!   `!Input` binding on the canvas goes dead while it holds focus, so typing `p` inserts a
//!   `p` instead of toggling the panel shut.

pub(super) mod connection_form;
pub(crate) mod entry;
mod list;
pub(super) mod workspace_form;

use std::collections::BTreeSet;

use gpui_kit::TestSupportExt;
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Subscription, Window, div, transparent_black,
};

use crate::database::Database;
use crate::settings::Settings;
use connection_form::{Committed, ConnectionForm, Probe};
use entry::Entry;
use workspace_form::WorkspaceForm;

/// Where the panel is in the reference's push navigation. The list stays mounted underneath: a
/// form is a view *of* the panel, not a second overlay stacked on it, which is what makes
/// escape mean "back" rather than "close everything".
enum View {
    List,
    Connection(Box<ConnectionForm>),
    Workspace(Box<WorkspaceForm>),
}

/// What the panel asks the workspace to do.
///
/// An event rather than a call back through a handle: the workspace owns this entity, so
/// reaching into it from here — while it is the thing updating us — is a re-entrant borrow,
/// which gpui turns into a panic rather than a compile error. Events are delivered after the
/// update that emitted them, so the borrow is long gone.
#[derive(Debug, Clone)]
pub(crate) enum PickerEvent {
    Switch {
        workspace: String,
        connection: String,
    },
    /// A connection was renamed and its canvas already moved with it. The workspace follows
    /// only if this is the connection it has open.
    Renamed {
        workspace: String,
        from: String,
        to: String,
    },
    /// A connection's entry is gone. Its document is still on disk.
    Removed {
        workspace: String,
        connection: String,
    },
    WorkspaceRenamed {
        from: String,
        to: String,
    },
    WorkspaceRemoved {
        name: String,
    },
}

/// The panel is open, and this is everything that is only true while it is.
struct Open {
    view: View,
    query: Entity<InputState>,
    /// An index into the *filtered* rows. The reference's cursor walks connections only: a
    /// workspace header is never a landing spot, it just expands when the cursor enters it.
    cursor: usize,
    /// Collapsed workspaces. Stored as the exception rather than the rule so a workspace added
    /// while the panel is open starts expanded, as the reference's `openWorkspaces` default does.
    collapsed: BTreeSet<SharedString>,
    /// Whatever held focus when the panel opened — the canvas, on every path that gets here. A
    /// handle of our own would die with the panel and leave the window focused on nothing.
    restore_focus: Option<FocusHandle>,
    _query: Subscription,
}

pub(crate) struct PickerView {
    /// Which connection is open, pushed in by the workspace rather than read back out of it,
    /// for the borrow reason on [`PickerEvent`].
    current: Option<(SharedString, SharedString)>,
    open: Option<Open>,
}

impl EventEmitter<PickerEvent> for PickerView {}

impl std::fmt::Debug for PickerView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PickerView")
            .field("open", &self.open.is_some())
            .finish_non_exhaustive()
    }
}

impl PickerView {
    pub(crate) fn new() -> Self {
        Self {
            current: None,
            open: None,
        }
    }

    /// Tells the panel which connection is open, so it can mark the row and put the cursor on
    /// it. Called when the window opens and again on every switch.
    pub(crate) fn set_current(&mut self, workspace: &str, connection: &str) {
        self.current = Some((
            SharedString::from(workspace.to_string()),
            SharedString::from(connection.to_string()),
        ));
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The reference's trigger is a toggle (`setShowPopover(!showPopover)`), so pressing `p`
    /// twice closes what it opened.
    pub(crate) fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_some() {
            self.close(window, cx);
        } else {
            self.show(window, cx);
        }
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search workspaces & connections…"));
        // Re-filtering changes which row the cursor is over, so it goes back to the best match
        // rather than keeping an index into a list that no longer exists.
        let subscription =
            cx.subscribe_in(&query, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.reset_cursor(cx),
                InputEvent::PressEnter { .. } => {
                    if matches!(this.view(), Some(View::List)) {
                        this.confirm(window, cx);
                    }
                }
                _ => {}
            });

        let restore_focus = window.focused(cx);
        query.update(cx, |query, cx| query.focus(window, cx));
        self.open = Some(Open {
            view: View::List,
            query,
            cursor: 0,
            collapsed: BTreeSet::new(),
            restore_focus,
            _query: subscription,
        });
        self.cursor_to_open_connection(cx);
        cx.notify();
    }

    fn view(&self) -> Option<&View> {
        self.open.as_ref().map(|open| &open.view)
    }

    fn view_mut(&mut self) -> Option<&mut View> {
        self.open.as_mut().map(|open| &mut open.view)
    }

    /// Pushes a connection form; `editing` names an existing connection or opens a blank one.
    pub(super) fn edit_connection(
        &mut self,
        at: (&str, Option<&str>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = ConnectionForm::new(at, window, cx);
        self.push(View::Connection(Box::new(form)), window, cx);
    }

    pub(super) fn edit_workspace(
        &mut self,
        editing: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = WorkspaceForm::new(editing, window, cx);
        self.push(View::Workspace(Box::new(form)), window, cx);
    }

    fn push(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.view = view;
        // The first field takes focus, as the reference's `autoFocus` does — otherwise the
        // search box keeps it and typing filters a list nobody is looking at.
        match &open.view {
            View::Connection(form) => {
                let name = form.name.clone();
                name.update(cx, |name, cx| name.focus(window, cx));
            }
            View::Workspace(form) => {
                let name = form.name.clone();
                name.update(cx, |name, cx| name.focus(window, cx));
            }
            View::List => {}
        }
        cx.notify();
    }

    /// Escape inside a form pops back here rather than closing the panel, which is the
    /// reference's behaviour and the reason a form is a view instead of a second overlay.
    pub(super) fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.view = View::List;
        let query = open.query.clone();
        query.update(cx, |query, cx| query.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.open.take() else {
            return;
        };
        let restore = open.restore_focus.clone();
        // Dropped before focus moves, so the `Blur` that follows reaches no live subscription.
        drop(open);
        if let Some(handle) = restore {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// The reference opens with the cursor on the connection you are already in, so the first
    /// arrow key moves relative to where you are rather than to the top of the list.
    fn cursor_to_open_connection(&mut self, cx: &mut Context<Self>) {
        let Some((workspace, connection)) = self.current.clone() else {
            return;
        };
        let found = self
            .rows(cx)
            .iter()
            .position(|row| row.workspace == workspace && row.name == connection);
        if let (Some(open), Some(found)) = (self.open.as_mut(), found) {
            open.cursor = found;
        }
    }

    fn reset_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open.as_mut() {
            open.cursor = 0;
        }
        cx.notify();
    }

    /// The search field, while the panel is open.
    pub(super) fn query(&self) -> Option<&Entity<InputState>> {
        self.open.as_ref().map(|open| &open.query)
    }

    /// Whether anything is typed. Not the same as "has rows": an empty result still counts as
    /// searching, which is what lets the empty state say *why* it is empty.
    pub(super) fn is_searching(&self, cx: &App) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| !open.query.read(cx).value().trim().is_empty())
    }

    /// The row Enter would pick.
    pub(super) fn cursor_entry(&self, cx: &App) -> Option<Entry> {
        let cursor = self.open.as_ref()?.cursor;
        self.rows(cx).get(cursor).cloned()
    }

    /// The rows the panel is showing, filtered by whatever is in the search box and gathered
    /// under their workspaces, so this sequence is exactly what the cursor walks.
    pub(super) fn rows(&self, cx: &App) -> Vec<Entry> {
        let Some(open) = self.open.as_ref() else {
            return Vec::new();
        };
        let query = open.query.read(cx).value().to_string();
        let filtered = entry::filter(entry::entries(Settings::get(cx)), &query);
        entry::group(filtered)
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .collect()
    }

    pub(super) fn open_connection(&self) -> Option<&(SharedString, SharedString)> {
        self.current.as_ref()
    }

    /// Moves the cursor by `step`, clamped rather than wrapping — the reference's
    /// `Math.max(0, …)` / `Math.min(len - 1, …)`.
    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if count == 0 {
            return;
        }
        let last = count - 1;
        let moved = isize::try_from(open.cursor)
            .unwrap_or(0)
            .saturating_add(step);
        open.cursor = usize::try_from(moved).unwrap_or(0).min(last);
        cx.notify();
    }

    /// Turns the SSH sub-form on or off on the showing connection form.
    pub(super) fn toggle_tunnel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The fields are built before the form is borrowed: creating an `InputState` needs
        // the context, and the form lives inside it.
        let on = matches!(self.view(), Some(View::Connection(form)) if form.tunnel.is_some());
        let fields = (!on).then(|| connection_form::TunnelFields::new(None, window, cx));
        if let Some(View::Connection(form)) = self.view_mut() {
            form.tunnel = fields;
        }
        cx.notify();
    }

    /// The reference's Duplicate: the same connection under `"<name> copy"`.
    ///
    /// Unlike the reference it does **not** keep the same URL by accident — it keeps it on
    /// purpose, and the name is what identifies a connection here, so the copy is editable
    /// without the two of them shadowing each other.
    pub(super) fn duplicate_connection(at: (&str, &str), cx: &mut Context<Self>) {
        let Some(source) = Settings::get(cx).connection(at).cloned() else {
            return;
        };
        let workspace = at.0.to_string();
        let copy = peek_config::DatabaseConnection {
            name: format!("{} copy", source.name),
            ..source
        };
        let written = connection_form::write(cx, |config| config.add_connection(&workspace, copy));
        if let Err(message) = written {
            log::warn!("peek: could not duplicate the connection: {message}");
        }
        cx.notify();
    }

    pub(super) fn arm_remove(&mut self, cx: &mut Context<Self>) {
        match self.view_mut() {
            Some(View::Connection(form)) => form.confirming_remove = true,
            Some(View::Workspace(form)) => form.confirming_remove = true,
            _ => {}
        }
        cx.notify();
    }

    /// The platform file chooser, for the SSH identity key.
    pub(super) fn browse_for_key(window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(chosen))) = paths.await else {
                return;
            };
            let Some(path) = chosen
                .first()
                .map(|path| path.to_string_lossy().to_string())
            else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(View::Connection(form)) = this.view() else {
                    return;
                };
                let Some(field) = form.tunnel.as_ref().map(|fields| fields.key_path.clone()) else {
                    return;
                };
                field.update(cx, |field, cx| field.set_value(path, window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the connection the form describes, once, and reports whether it answered.
    ///
    /// It never touches the `Database` global: the canvas keeps querying whatever it was
    /// querying, which is the whole reason `Session::probe` exists.
    pub(super) fn probe_connection(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(View::Connection(form)) = self.view() else {
            return;
        };
        let connection = form.collect(cx);
        let Some(session) = Database::session(cx) else {
            return;
        };
        let tunnel = connection
            .ssh_tunnel
            .as_ref()
            .map(crate::database::tunnel_config);
        let pending = session.probe(
            connection.url,
            tunnel,
            peek_db::HostKeyPolicy::TrustOnFirstUse,
        );
        self.set_probe(Probe::Running, cx);

        cx.spawn(async move |this, cx| {
            let outcome = match pending.await {
                Ok(Ok(engine)) => Probe::Reached(engine),
                Ok(Err(error)) => Probe::Failed(SharedString::from(error.to_string())),
                Err(_) => Probe::Failed(SharedString::from("the database runtime stopped")),
            };
            let _ = this.update(cx, |this, cx| this.set_probe(outcome, cx));
        })
        .detach();
    }

    fn set_probe(&mut self, probe: Probe, cx: &mut Context<Self>) {
        if let Some(View::Connection(form)) = self.view_mut() {
            form.probe = probe;
        }
        cx.notify();
    }

    /// Commits whichever form is showing.
    ///
    /// A refusal — a duplicate name, a read-only run, a canvas that could not be moved — stays
    /// in the form next to the field that caused it, rather than closing and losing what was
    /// typed.
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = Settings::persistence(cx);
        let outcome = match self.view() {
            Some(View::Connection(form)) if form.can_save(cx) => {
                let workspace = form.workspace.to_string();
                connection_form::commit(form, mode, cx).map(|committed| match committed {
                    Committed::Renamed { from, to } => Some(PickerEvent::Renamed {
                        workspace,
                        from,
                        to,
                    }),
                    Committed::Added | Committed::Updated => None,
                })
            }
            Some(View::Workspace(form)) if form.can_save(cx) => {
                workspace_form::commit(form, mode, cx).map(|renamed| {
                    renamed.map(|(from, to)| PickerEvent::WorkspaceRenamed { from, to })
                })
            }
            _ => return,
        };

        match outcome {
            Ok(event) => {
                if let Some(event) = event {
                    cx.emit(event);
                }
                self.back_to_list(window, cx);
            }
            Err(message) => self.set_form_error(Some(message), cx),
        }
    }

    /// Removes whatever the showing form is editing, and tells the workspace so it can repoint
    /// if it was looking at it. Documents are left on disk: only the entry goes.
    pub(super) fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let event = match self.view() {
            Some(View::Connection(form)) => {
                let Some(connection) = form.editing.clone() else {
                    return;
                };
                let workspace = form.workspace.to_string();
                let at = (workspace.clone(), connection.to_string());
                let removed = connection_form::write(cx, |config| {
                    config.remove_connection((&at.0, &at.1));
                    Ok(())
                });
                if let Err(message) = removed {
                    self.set_form_error(Some(message), cx);
                    return;
                }
                PickerEvent::Removed {
                    workspace,
                    connection: connection.to_string(),
                }
            }
            Some(View::Workspace(form)) => {
                let Some(name) = form.editing.clone().map(|name| name.to_string()) else {
                    return;
                };
                let removed = connection_form::write(cx, |config| {
                    config.remove_workspace(&name);
                    Ok(())
                });
                if let Err(message) = removed {
                    self.set_form_error(Some(message), cx);
                    return;
                }
                PickerEvent::WorkspaceRemoved { name }
            }
            _ => return,
        };
        cx.emit(event);
        self.back_to_list(window, cx);
    }

    fn set_form_error(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
        match self.view_mut() {
            Some(View::Connection(form)) => form.error = message,
            Some(View::Workspace(form)) => form.error = message,
            _ => {}
        }
        cx.notify();
    }

    /// Switches to the row under the cursor. Everything that makes a switch a switch —
    /// flushing autosave, rebuilding the canvas, reconnecting the database — belongs to
    /// [`WorkspaceView::switch_connection`]; this only names the connection.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.open.as_ref().map_or(0, |open| open.cursor);
        let Some(row) = self.rows(cx).get(cursor).cloned() else {
            return;
        };
        self.switch_to(&row.workspace, &row.name, window, cx);
    }

    pub(super) fn switch_to(
        &mut self,
        workspace: &SharedString,
        connection: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (workspace, connection) = (workspace.to_string(), connection.to_string());
        self.close(window, cx);
        cx.emit(PickerEvent::Switch {
            workspace,
            connection,
        });
    }

    pub(super) fn toggle_workspace(&mut self, workspace: &SharedString, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if !open.collapsed.remove(workspace) {
            open.collapsed.insert(workspace.clone());
        }
        cx.notify();
    }

    /// Whether a workspace's connections are showing.
    ///
    /// While searching, everything is expanded whatever the collapsed set says: a result you
    /// cannot see is not a result. That is the reference's behaviour too.
    pub(super) fn is_expanded(&self, workspace: &SharedString, searching: bool) -> bool {
        searching
            || self
                .open
                .as_ref()
                .is_none_or(|open| !open.collapsed.contains(workspace))
    }

    /// `up` / `down` / `escape`, taken on the panel root rather than bound as actions.
    ///
    /// This works because a *single-line* `InputState` registers no `MoveUp`/`MoveDown`
    /// listener — gpui-base gates those on `is_multi_line()` — so the keystroke finds no
    /// handler, propagation continues, and this ancestor sees it. Escape arrives the same way:
    /// the input's own handler propagates when there is nothing to clear, which
    /// `title_bar/pages.rs` already relies on.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_none() {
            return;
        }
        let in_list = matches!(self.view(), Some(View::List));
        match event.keystroke.key.as_str() {
            "down" if in_list => self.step(1, cx),
            "up" if in_list => self.step(-1, cx),
            // `mod-enter` saves a form, as the reference's `getHotkeyHandler` does. Plain
            // enter belongs to the field the caret is in.
            "enter" if !in_list && event.keystroke.modifiers.platform => self.save(window, cx),
            "escape" => match self.view() {
                Some(View::List) | None => self.close(window, cx),
                Some(_) => self.back_to_list(window, cx),
            },
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for PickerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A closed picker still renders, as an empty layer: it is an entity child, so it
        // repaints on its own `notify` without the workspace having to observe it.
        let layer = div().id("connection-picker-layer").absolute().inset_0();
        if !self.is_open() {
            return layer.invisible();
        }
        layer
            .child(scrim(cx))
            .child(list::panel(self, window, cx))
            .on_key_down(cx.listener(Self::on_key_down))
    }
}

/// A full-window sibling that swallows the press that dismisses the panel. It paints nothing:
/// the reference dims nothing behind its popover, and `occlude` is a hitbox property rather
/// than a painted one, so a transparent layer still blocks.
fn scrim(cx: &mut Context<PickerView>) -> impl IntoElement {
    div()
        .id("connection-picker-scrim")
        .test_support()
        .absolute()
        .inset_0()
        .occlude()
        .bg(transparent_black())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| this.close(window, cx)),
        )
}
