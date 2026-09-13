use super::super::{AGENT_NODE, CANVAS_NOT_TYPING, Command, Group, actions, one_selected_agent};

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Agent::Fork",
        title: "Fork conversation",
        label: None,
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
        label: None,
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
        label: None,
        group: Group::Agent,
        keywords: "cancel halt interrupt turn",
        default_keys: &[],
        context: AGENT_NODE,
        build: || Box::new(actions::agent::Stop),
        available: one_selected_agent,
    },
];
