//! One logical command = one gpui `Action` = one entry here. Keyboard shortcuts, the command
//! palette, menus and buttons all dispatch the same action; MCP, multiplayer and undo use the
//! `peek_canvas::Document` mutation API directly and never synthesize actions.

pub mod actions;
pub mod keymap;

use gpui_kit::Action;
use peek_canvas::Scope;

/// Key contexts and the binding predicates over them. Constants rather than an enum: they are
/// pasted straight into gpui bindings, `key_context` calls and tooltips.
///
/// The two are not interchangeable, and gpui does not tell you so kindly. A *context* — the
/// bare identifiers an element publishes — is all `KeyContext::parse` accepts, which is what
/// `key_context` and every tooltip that displays a shortcut take. Hand it a *predicate*
/// instead and its parser reaches the `&`, consumes nothing, and recurses on the same input
/// until the stack is gone: a crash, not a parse error. So a name without `_NOT_TYPING` is a
/// context and may go anywhere; one with it is a predicate and belongs only in a binding.
pub const WORKSPACE: &str = "Workspace";
pub const WORKSPACE_NOT_TYPING: &str = "Workspace && !Input && !NumberInput && !JumpMode";
pub const CANVAS: &str = "Canvas";
pub const CANVAS_NOT_TYPING: &str = "Canvas && !Input && !NumberInput && !JumpMode";
/// The canvas while the jump overlay owns the keyboard. Adding the identifier alongside
/// `Canvas` is what makes every `!JumpMode` binding above go dead for the duration, so a
/// letter picks a label instead of arming a tool — no second focus handle needed.
pub const CANVAS_JUMPING: &str = "Canvas JumpMode";
/// A query node's own context. Commands bound here fire while the SQL editor holds focus,
/// which is why they carry a modifier: a bare key would be swallowed by typing.
pub const QUERY_NODE: &str = "QueryNode";
/// A result node's own context, for the commands that act on the table's selection while the
/// table holds focus.
pub const RESULT_NODE: &str = "ResultNode";
pub const AGENT_NODE: &str = "AgentNode";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Tool,
    Agent,
    Query,
    Edit,
    History,
    Zoom,
    Page,
    View,
    App,
}

impl Group {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Tool => "Tools",
            Self::Agent => "Agent",
            Self::Query => "Query",
            Self::Edit => "Edit",
            Self::History => "History",
            Self::Zoom => "Zoom",
            Self::Page => "Pages",
            Self::View => "View",
            Self::App => "Application",
        }
    }
}

#[derive(Debug)]
pub struct Command {
    /// Equals the gpui action name and the `settings.json` keymap value, e.g. `"Zoom::FitView"`.
    pub id: &'static str,
    pub title: &'static str,
    pub group: Group,
    /// Extra palette search terms.
    pub keywords: &'static str,
    /// Peek keymap syntax (`docs/keymap.md`); translated to gpui keystrokes at bind time.
    pub default_keys: &'static [&'static str],
    pub context: &'static str,
    pub build: fn() -> Box<dyn Action>,
    pub available: fn(&Scope) -> bool,
}

fn always(_: &Scope) -> bool {
    true
}

fn several_pages(scope: &Scope) -> bool {
    scope.pages > 1
}

fn has_selection(scope: &Scope) -> bool {
    scope.selected + scope.selected_edges > 0
}

/// The result table's commands need a result node to act on.
fn has_selected_results(scope: &Scope) -> bool {
    scope.selected_results > 0
}

/// Running needs a query to run and a database to run it against.
fn can_run_queries(scope: &Scope) -> bool {
    scope.selected_queries > 0 && scope.connected
}

/// Framing needs bounds, and only nodes have them.
fn has_selected_nodes(scope: &Scope) -> bool {
    scope.selected > 0
}

fn has_selected_queries(scope: &Scope) -> bool {
    scope.selected_queries > 0
}

/// Entering a query's editor needs one unambiguous query to enter.
fn one_selected_query(scope: &Scope) -> bool {
    scope.selected == 1 && scope.selected_queries == 1
}

/// Forking, stopping and cycling modes all act on one unambiguous agent node.
fn one_selected_agent(scope: &Scope) -> bool {
    scope.selected == 1 && scope.selected_agents == 1
}

fn can_undo(scope: &Scope) -> bool {
    scope.history.can_undo
}

fn can_redo(scope: &Scope) -> bool {
    scope.history.can_redo
}

/// Every command this build implements. Later milestones append here; a user keymap entry
/// naming an id that is not (yet) listed is logged and skipped.
pub static COMMANDS: &[Command] = &[
    Command {
        id: "Zoom::In",
        title: "Zoom in",
        group: Group::Zoom,
        keywords: "bigger closer",
        default_keys: &["meta-="],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::zoom::In),
        available: always,
    },
    Command {
        id: "Zoom::Out",
        title: "Zoom out",
        group: Group::Zoom,
        keywords: "smaller further",
        default_keys: &["meta--"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::zoom::Out),
        available: always,
    },
    Command {
        id: "Zoom::Reset",
        title: "Reset zoom to 100%",
        group: Group::Zoom,
        keywords: "actual size",
        default_keys: &["meta-0"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::zoom::Reset),
        available: always,
    },
    Command {
        id: "Zoom::FitView",
        title: "Fit all nodes in view",
        group: Group::Zoom,
        keywords: "fit view frame everything",
        default_keys: &["meta-shift-0"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::zoom::FitView),
        available: always,
    },
    Command {
        id: "Zoom::FitSelection",
        title: "Fit selected nodes in view",
        group: Group::Zoom,
        keywords: "frame selection",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::zoom::FitSelection),
        available: has_selected_nodes,
    },
    Command {
        id: "Edit::SelectAll",
        title: "Select all nodes",
        group: Group::Edit,
        keywords: "",
        default_keys: &["meta-a"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::SelectAll),
        available: always,
    },
    Command {
        id: "Tool::Select",
        title: "Clear selection",
        group: Group::Tool,
        keywords: "deselect escape",
        default_keys: &["escape"],
        // The bare context, not the `!Input` predicate: escape has to disarm a tool and cancel
        // jump mode while an editor holds focus too.
        context: CANVAS,
        build: || Box::new(actions::tool::Select),
        available: always,
    },
    Command {
        id: "Edit::DeleteSelection",
        title: "Delete selection",
        group: Group::Edit,
        keywords: "remove backspace node edge",
        default_keys: &["backspace"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::edit::DeleteSelection),
        available: has_selection,
    },
    Command {
        id: "History::Undo",
        title: "Undo",
        group: Group::History,
        keywords: "revert back",
        default_keys: &["meta-z"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::history::Undo),
        available: can_undo,
    },
    Command {
        id: "History::Redo",
        title: "Redo",
        group: Group::History,
        keywords: "again forward",
        default_keys: &["meta-shift-z"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::history::Redo),
        available: can_redo,
    },
    Command {
        id: "Tool::Query",
        title: "New query node",
        group: Group::Tool,
        keywords: "place add sql select",
        default_keys: &["q"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::tool::Query),
        available: always,
    },
    Command {
        id: "Tool::Agent",
        title: "New agent node",
        group: Group::Tool,
        keywords: "place add ai chat llm assistant",
        default_keys: &["a"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::tool::Agent),
        available: always,
    },
    Command {
        id: "Agent::Fork",
        title: "Fork conversation",
        group: Group::Agent,
        keywords: "branch copy duplicate agent chat",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::agent::Fork),
        available: one_selected_agent,
    },
    Command {
        id: "Agent::CycleMode",
        title: "Cycle agent mode",
        group: Group::Agent,
        keywords: "acp plan accept edits switch",
        // Fires while the composer holds focus, which is where it is pressed.
        default_keys: &["shift-tab"],
        context: AGENT_NODE,
        build: || Box::new(actions::agent::CycleMode),
        available: one_selected_agent,
    },
    Command {
        id: "Agent::Stop",
        title: "Stop the agent",
        group: Group::Agent,
        keywords: "cancel halt interrupt turn",
        default_keys: &[],
        context: AGENT_NODE,
        build: || Box::new(actions::agent::Stop),
        available: one_selected_agent,
    },
    Command {
        id: "Page::Search",
        title: "Find in result",
        group: Group::Page,
        keywords: "search filter find rows",
        default_keys: &["meta-f"],
        // The reference binds this page-wide and lets a selected result node intercept it. Page
        // search does not exist here yet, so for now it belongs to the result node outright;
        // when page search lands this moves to `CANVAS_NOT_TYPING` and the node keeps its own handler.
        context: RESULT_NODE,
        build: || Box::new(actions::page::Search),
        available: has_selected_results,
    },
    Command {
        id: "Edit::Copy",
        title: "Copy selection",
        group: Group::Edit,
        keywords: "clipboard cells rows tsv",
        default_keys: &["meta-c"],
        context: RESULT_NODE,
        build: || Box::new(actions::edit::copy::Copy),
        available: has_selected_results,
    },
    Command {
        id: "Query::Run",
        title: "Run query",
        group: Group::Query,
        keywords: "execute sql send",
        // `cmd-enter`, the binding `QueryNode.tsx` registers inside its editor. Bound on the
        // node context so it fires while the editor holds focus, which is where it is pressed.
        default_keys: &["meta-enter"],
        context: QUERY_NODE,
        build: || Box::new(actions::query::Run),
        available: can_run_queries,
    },
    Command {
        id: "Query::Format",
        title: "Format query",
        group: Group::Query,
        keywords: "pretty print indent sql",
        default_keys: &["meta-s"],
        context: QUERY_NODE,
        build: || Box::new(actions::query::Format),
        available: has_selected_queries,
    },
    Command {
        id: "Tool::Text",
        title: "New text node",
        group: Group::Tool,
        keywords: "place add note label",
        default_keys: &["t"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::tool::Text),
        available: always,
    },
    Command {
        id: "Tool::Variable",
        title: "New variable node",
        group: Group::Tool,
        keywords: "place add vars parameter",
        default_keys: &["v"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::tool::Variable),
        available: always,
    },
    Command {
        id: "Tool::Draw",
        title: "Draw",
        group: Group::Tool,
        keywords: "pen freehand sketch stroke ink annotate",
        default_keys: &["d"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::tool::Draw),
        available: always,
    },
    Command {
        id: "Page::New",
        title: "New page",
        group: Group::Page,
        keywords: "add tab",
        default_keys: &["meta-t"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::New),
        available: always,
    },
    Command {
        id: "Page::Close",
        title: "Close the current page",
        group: Group::Page,
        keywords: "delete remove page",
        default_keys: &["meta-w"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::Close),
        available: several_pages,
    },
    Command {
        id: "Page::Previous",
        title: "Previous page",
        group: Group::Page,
        keywords: "tab left",
        default_keys: &["meta-shift-["],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::Previous),
        available: several_pages,
    },
    Command {
        id: "Page::Next",
        title: "Next page",
        group: Group::Page,
        keywords: "tab right",
        default_keys: &["meta-shift-]"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::Next),
        available: several_pages,
    },
    Command {
        id: "Page::GoToNode",
        title: "Go to a node",
        group: Group::Page,
        keywords: "go to jump navigate node label hint keyboard",
        default_keys: &["g"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::GoToNode),
        available: always,
    },
    Command {
        id: "Page::SelectNodeLeft",
        title: "Select node to the left",
        group: Group::Page,
        keywords: "select node move left arrow navigate keyboard",
        default_keys: &["meta-arrowleft"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::SelectNodeLeft),
        available: always,
    },
    Command {
        id: "Page::SelectNodeRight",
        title: "Select node to the right",
        group: Group::Page,
        keywords: "select node move right arrow navigate keyboard",
        default_keys: &["meta-arrowright"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::SelectNodeRight),
        available: always,
    },
    Command {
        id: "Page::SelectNodeUp",
        title: "Select node above",
        group: Group::Page,
        keywords: "select node move up arrow navigate keyboard",
        default_keys: &["meta-arrowup"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::SelectNodeUp),
        available: always,
    },
    Command {
        id: "Page::SelectNodeDown",
        title: "Select node below",
        group: Group::Page,
        keywords: "select node move down arrow navigate keyboard",
        default_keys: &["meta-arrowdown"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::page::SelectNodeDown),
        available: always,
    },
    Command {
        id: "Query::Focus",
        title: "Edit the selected query",
        group: Group::Query,
        keywords: "enter edit focus query editor sql",
        default_keys: &["enter"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::query::Focus),
        available: one_selected_query,
    },
    Command {
        id: "View::ToggleCameraLock",
        title: "Lock or unlock the camera",
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
        group: Group::View,
        keywords: "chrome focus mode",
        default_keys: &["meta-."],
        context: WORKSPACE,
        build: || Box::new(actions::view::ToggleUi),
        available: always,
    },
    Command {
        id: "CommandPalette::Open",
        title: "Command palette",
        group: Group::App,
        keywords: "",
        default_keys: &["meta-p", "meta-shift-p"],
        context: WORKSPACE,
        build: || Box::new(actions::command_palette::Open),
        available: always,
    },
    Command {
        id: "Theme::Open",
        title: "Change theme",
        group: Group::View,
        keywords: "colors appearance dark light",
        default_keys: &[],
        context: WORKSPACE,
        build: || Box::new(actions::theme::Open),
        available: always,
    },
    Command {
        id: "App::Quit",
        title: "Quit Peek",
        group: Group::App,
        keywords: "exit",
        default_keys: &["meta-q"],
        context: WORKSPACE,
        build: || Box::new(actions::app::Quit),
        available: always,
    },
];

#[must_use]
pub fn find(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_match_action_names_and_are_unique() {
        let mut seen = HashSet::new();
        for command in COMMANDS {
            assert_eq!((command.build)().name(), command.id, "{}", command.id);
            assert!(seen.insert(command.id), "duplicate id {}", command.id);
        }
    }

    /// Running needs somewhere to run. Without this the palette would offer "Run query" on a
    /// canvas with no connection, and the action would silently do nothing.
    #[test]
    fn run_query_needs_both_a_query_and_a_connection() {
        let command = find("Query::Run").expect("Query::Run is registered");
        let selected = Scope {
            selected_queries: 1,
            ..Scope::default()
        };
        assert!(!(command.available)(&selected), "no connection");

        let connected = Scope {
            connected: true,
            ..Scope::default()
        };
        assert!(!(command.available)(&connected), "no query selected");

        let both = Scope {
            selected_queries: 1,
            connected: true,
            ..Scope::default()
        };
        assert!((command.available)(&both));
    }

    /// `cmd-enter` is bound on the node context, not the canvas, so it fires while the SQL
    /// editor holds focus — which is where it is actually pressed.
    #[test]
    fn run_query_is_bound_where_the_editor_has_focus() {
        let command = find("Query::Run").expect("Query::Run is registered");
        assert_eq!(command.context, QUERY_NODE);
        assert_eq!(command.default_keys, &["meta-enter"]);
    }

    /// A bare printable key must never fire while an editor holds focus, or it is impossible
    /// to type that letter into a query, text or variable node.
    #[test]
    fn unmodified_letter_keys_do_not_fire_while_typing() {
        for command in COMMANDS {
            for combo in command.default_keys {
                let bare_letter = combo.len() == 1
                    && combo
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric());
                assert!(
                    !bare_letter || command.context.contains("!Input"),
                    "{}: {combo} fires while typing",
                    command.id
                );
            }
        }
    }

    #[test]
    fn default_keys_translate() {
        for command in COMMANDS {
            for combo in command.default_keys {
                peek_config::gpui_keystroke(combo)
                    .unwrap_or_else(|error| panic!("{}: {combo}: {error}", command.id));
            }
        }
    }
}
