//! One logical command = one gpui `Action` = one entry here. Keyboard shortcuts, the command
//! palette, menus and buttons all dispatch the same action; MCP, multiplayer and undo use the
//! `peek_canvas::Document` mutation API directly and never synthesize actions.
//!
//! The entries themselves live in [`registry`], one file per group.

pub mod actions;
pub mod keymap;
pub mod palette;
mod registry;

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
/// The SQL editor inside a query node, for the one binding gpui-kit already claims.
///
/// `Input` binds `secondary-enter` — `cmd-enter` on macOS — to its own `Enter`, which in a
/// multi-line editor inserts a newline. Bindings resolve by how deep in the context stack they
/// match, and `Input` is the deepest context the editor publishes, so a bare `QueryNode` loses
/// the keystroke. Naming `Input` reaches that depth too, and ties at equal depth go to the
/// binding registered last — ours, installed after `gpui_kit::init`.
pub const QUERY_EDITOR: &str = "QueryNode > Input";
/// A result node's own context, for the commands that act on the table's selection while the
/// table holds focus.
pub const RESULT_NODE: &str = "ResultNode";
pub const AGENT_NODE: &str = "AgentNode";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Tool,
    Agent,
    Query,
    Result,
    Edit,
    History,
    Zoom,
    Page,
    Region,
    View,
    Export,
    Settings,
    Help,
    App,
}

impl Group {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Tool => "Tools",
            Self::Agent => "Agent",
            Self::Query => "Query",
            Self::Result => "Result",
            Self::Edit => "Edit",
            Self::History => "History",
            Self::Zoom => "Zoom",
            Self::Page => "Pages",
            Self::Region => "Regions",
            Self::View => "View",
            Self::Export => "Export",
            Self::Settings => "Settings",
            Self::Help => "Help",
            Self::App => "Application",
        }
    }
}

#[derive(Debug)]
pub struct Command {
    /// Equals the gpui action name and the `settings.json` keymap value, e.g. `"Zoom::FitView"`.
    pub id: &'static str,
    /// The stable name, used by tooltips and the keymap modal.
    pub title: &'static str,
    /// The palette label when it depends on state — "Hide UI" against "Show UI". `None` means
    /// the label is just the title.
    pub label: Option<fn(&Scope) -> &'static str>,
    pub group: Group,
    /// Extra palette search terms.
    pub keywords: &'static str,
    /// Peek keymap syntax (`docs/keymap.md`); translated to gpui keystrokes at bind time.
    pub default_keys: &'static [&'static str],
    pub context: &'static str,
    pub build: fn() -> Box<dyn Action>,
    pub available: fn(&Scope) -> bool,
}

impl Command {
    /// What the palette shows for this command right now.
    #[must_use]
    pub fn label(&self, scope: &Scope) -> &'static str {
        self.label.map_or(self.title, |label| label(scope))
    }
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

/// Rerunning the whole page needs something to rerun and a database to rerun it against.
/// Unlike [`can_run_queries`] it does not care what is selected.
fn can_rerun_page(scope: &Scope) -> bool {
    scope.queries > 0 && scope.connected
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

/// Every command this build implements, in group order. Later milestones append to the group
/// file they belong to; a user keymap entry naming an id that is not (yet) listed is logged
/// and skipped.
pub fn all() -> impl Iterator<Item = &'static Command> {
    registry::GROUPS.iter().copied().flatten()
}

#[must_use]
pub fn find(id: &str) -> Option<&'static Command> {
    all().find(|command| command.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_match_action_names_and_are_unique() {
        let mut seen = HashSet::new();
        for command in all() {
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

    /// `cmd-enter` is bound on the editor, not the canvas, so it fires while the SQL editor
    /// holds focus — which is where it is actually pressed. It has to name `Input` to outrank
    /// gpui-kit's own `secondary-enter`, which would otherwise insert a newline instead.
    #[test]
    fn run_query_is_bound_where_the_editor_has_focus() {
        let command = find("Query::Run").expect("Query::Run is registered");
        assert_eq!(command.context, QUERY_EDITOR);
        assert_eq!(command.default_keys, &["meta-enter"]);
    }

    /// Every registry context has to parse as a predicate, or `keymap::bind` panics on start-up.
    #[test]
    fn contexts_parse_as_predicates() {
        for command in all() {
            gpui_kit::KeyBindingContextPredicate::parse(command.context)
                .unwrap_or_else(|error| panic!("{}: {}: {error}", command.id, command.context));
        }
    }

    /// A bare printable key must never fire while an editor holds focus, or it is impossible
    /// to type that letter into a query, text or variable node.
    #[test]
    fn unmodified_letter_keys_do_not_fire_while_typing() {
        for command in all() {
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
        for command in all() {
            for combo in command.default_keys {
                peek_config::gpui_keystroke(combo)
                    .unwrap_or_else(|error| panic!("{}: {combo}: {error}", command.id));
            }
        }
    }

    /// The one data-carrying action. It has no registry entry, so nothing else checks that its
    /// name still matches the id the palette and any future keymap entry would use.
    #[test]
    fn go_to_page_is_named_like_every_other_action() {
        let action = actions::page::GoTo {
            page: peek_document::PageId::from("page-1"),
        };
        assert_eq!(action.name(), "Page::GoTo");
    }

    /// A stateful label has to actually change, or it is just a `title` with extra steps.
    #[test]
    fn stateful_labels_follow_the_scope() {
        let command = find("View::ToggleUi").expect("View::ToggleUi is registered");
        let visible = Scope::default();
        let hidden = Scope {
            chrome_hidden: true,
            ..Scope::default()
        };
        assert_ne!(command.label(&visible), command.label(&hidden));
        assert_eq!(command.label(&visible), "Hide UI");
    }
}
