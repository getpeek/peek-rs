//! What goes in each menu, and raising it.
//!
//! `CellContextMenu.tsx`'s three modes and `ResultHeaderMenu.tsx`'s two items. Every leaf is a
//! registered command, so this builds a list of labelled actions and hands it to the canvas —
//! which is what lets the menu be drawn outside the node without borrowing it.
//!
//! Items are **conditionally present, never disabled**: neither reference menu has a disabled
//! state, and an item that cannot do anything is better left out than greyed out.

use gpui_kit::assets::IconName;
use gpui_kit::{Action, App, Context, Pixels, Point as WindowPoint, SharedString};

use super::super::ResultTable;
use super::actions::Format;
use super::scope::{MenuTarget, Scope};
use crate::canvas::context_menu::{
    CELL_MENU_WIDTH, HEADER_MENU_WIDTH, MenuEntry, MenuItem, MenuState,
};
use crate::commands::actions;

/// How much of a value the `Copy "…"` label shows before it gives up
/// (`MAX_COPY_VALUE_LENGTH`). The ellipsis goes **inside** the quote, as it does there.
const MAX_LABEL_VALUE: usize = 13;

impl ResultTable {
    /// Raises the cell menu for a right-click at `at`, in pane coordinates.
    pub(crate) fn open_cell_menu(
        &mut self,
        target: MenuTarget,
        at: WindowPoint<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let entries = self.arm(target, cx).then(|| self.cell_entries(cx));
        if let Some(entries) = entries {
            self.raise(entries, at, CELL_MENU_WIDTH, cx);
        }
    }

    pub(crate) fn open_header_menu(
        &mut self,
        target: MenuTarget,
        at: WindowPoint<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let entries = self
            .arm(target, cx)
            .then(|| self.header_entries(target.column, cx));
        if let Some(entries) = entries {
            self.raise(entries, at, HEADER_MENU_WIDTH, cx);
        }
    }

    /// Records what was clicked and claims the node, returning whether there is a menu worth
    /// raising at all.
    ///
    /// The commands find their table through the **canvas selection**, and a right press does
    /// not select a node the way a left one does — so the menu has to say which node it came
    /// from, exactly as the toolbar's buttons do.
    fn arm(&mut self, target: MenuTarget, cx: &mut Context<Self>) -> bool {
        if self.table.read(cx).delegate().result_rows().is_empty() {
            return false;
        }
        self.table.update(cx, |table, _| {
            table.delegate_mut().set_menu_target(Some(target));
        });
        self.select_self(cx);
        true
    }

    fn raise(
        &mut self,
        entries: Vec<MenuEntry>,
        at: WindowPoint<Pixels>,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(canvas) = self.canvas.upgrade() else {
            return;
        };
        canvas.update(cx, |canvas, cx| {
            // The press arrives in window coordinates; the overlay is placed inside the canvas
            // pane, which the title bar offsets.
            let at = canvas.to_pane(at);
            canvas.open_context_menu(
                MenuState {
                    at,
                    width,
                    entries,
                    open_submenu: None,
                },
                cx,
            );
        });
    }

    fn cell_entries(&self, cx: &App) -> Vec<MenuEntry> {
        let scope = self.scope(cx);
        let mut entries = Vec::new();

        // A rectangle only offers a variable when it spans one column: two columns have no
        // single list of values to put in an `IN (…)`.
        if scope.variable_column().is_some() {
            entries.push(item(
                self.id("variable"),
                "Use as variable",
                IconName::AtSign,
                Box::new(actions::result::UseAsVariable),
            ));
        }
        if let Scope::Cell(row, column) = scope {
            entries.push(item(
                self.id("copy-value"),
                self.copy_value_label(row, column, cx),
                IconName::Copy,
                Box::new(actions::result::CopyValue),
            ));
        }
        entries.push(self.formats("copy", copy_label(&scope), IconName::Copy, copy_action));
        entries.push(self.formats(
            "export",
            export_label(&scope),
            IconName::Download,
            export_action,
        ));

        // Deleting is offered only for a selected row in a writable result, so the one
        // irreversible item in the menu never sits under a stray right-click.
        if let Some(count) = self.deletable_rows(cx) {
            entries.push(MenuEntry::Separator);
            entries.push(MenuEntry::Item(Box::new(
                MenuItem::command(
                    self.id("delete"),
                    if count == 1 {
                        "Delete row".to_string()
                    } else {
                        format!("Delete {count} rows")
                    },
                    IconName::Trash,
                    Box::new(actions::result::DeleteRows),
                )
                .danger(),
            )));
        }
        entries
    }

    fn header_entries(&self, column: usize, cx: &App) -> Vec<MenuEntry> {
        let table = self.table.read(cx);
        let name = table
            .delegate()
            .result_rows()
            .columns()
            .get(column)
            .map_or_else(String::new, |column| column.name.clone());

        vec![
            MenuEntry::Label(SharedString::from(name)),
            item(
                self.id("variable"),
                "Use as variable",
                IconName::AtSign,
                Box::new(actions::result::UseAsVariable),
            ),
            self.formats(
                "export",
                "Export column".to_string(),
                IconName::Download,
                export_action,
            ),
        ]
    }

    /// A heading over the three formats, which open beside it on hover.
    fn formats(
        &self,
        prefix: &str,
        label: String,
        icon: IconName,
        action: fn(Format) -> Box<dyn Action>,
    ) -> MenuEntry {
        let children = [Format::Json, Format::Csv, Format::Sql]
            .into_iter()
            .map(|format| {
                MenuItem::command(
                    self.id(&format!("{prefix}-{}", format.extension())),
                    format.label(),
                    format_icon(format),
                    action(format),
                )
            })
            .collect();
        MenuEntry::Item(Box::new(MenuItem::submenu(
            self.id(prefix),
            label,
            icon,
            children,
        )))
    }

    /// Element ids are derived from the node, so two result nodes never collide.
    fn id(&self, suffix: &str) -> SharedString {
        SharedString::from(format!("{}-menu-{suffix}", self.node))
    }

    /// `Copy "Lola Stark"`, cut at 13 characters with the ellipsis inside the quote.
    fn copy_value_label(&self, row: usize, column: usize, cx: &App) -> String {
        let text = self
            .table
            .read(cx)
            .delegate()
            .result_rows()
            .cell(row, column)
            .map(display)
            .unwrap_or_default();
        if text.chars().count() > MAX_LABEL_VALUE {
            let cut: String = text.chars().take(MAX_LABEL_VALUE).collect();
            return format!("Copy \"{cut}…\"");
        }
        format!("Copy \"{text}\"")
    }
}

/// `stringifyValue`'s rule: a NULL is the empty string, so `Copy ""` is a real label.
fn display(cell: &peek_document::Cell) -> String {
    if cell.is_null() {
        String::new()
    } else {
        cell.to_display_string()
    }
}

fn item(
    id: SharedString,
    label: impl Into<SharedString>,
    icon: IconName,
    action: Box<dyn Action>,
) -> MenuEntry {
    MenuEntry::Item(Box::new(MenuItem::command(id, label, icon, action)))
}

fn copy_action(format: Format) -> Box<dyn Action> {
    match format {
        Format::Json => Box::new(actions::result::CopyAsJson),
        Format::Csv => Box::new(actions::result::CopyAsCsv),
        Format::Sql => Box::new(actions::result::CopyAsSql),
    }
}

fn export_action(format: Format) -> Box<dyn Action> {
    match format {
        Format::Json => Box::new(actions::result::ExportAsJson),
        Format::Csv => Box::new(actions::result::ExportAsCsv),
        Format::Sql => Box::new(actions::result::ExportAsSql),
    }
}

const fn format_icon(format: Format) -> IconName {
    match format {
        Format::Json => IconName::Braces,
        Format::Csv => IconName::Table,
        Format::Sql => IconName::Database,
    }
}

fn copy_label(scope: &Scope) -> String {
    match scope {
        Scope::Cells(_) => "Copy selection".to_string(),
        Scope::Rows(rows) => format!("Copy {} rows", rows.len()),
        Scope::Column(_) => "Copy column".to_string(),
        Scope::Cell(..) | Scope::Whole => "Copy row".to_string(),
    }
}

fn export_label(scope: &Scope) -> String {
    match scope {
        Scope::Cells(_) => "Export selection".to_string(),
        Scope::Rows(rows) => format!("Export {} rows", rows.len()),
        Scope::Column(_) => "Export column".to_string(),
        Scope::Cell(..) | Scope::Whole => "Export row".to_string(),
    }
}
