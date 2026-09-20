use super::super::{CANVAS_NOT_TYPING, Command, Group, actions, always};
use peek_canvas::Scope;
use peek_canvas::regions::grouping;

/// The reference's row reads `Add 3 nodes to "Churn"`. `label` is handed a `Scope` — a `Copy`
/// struct of counters, which is what keeps this registry free of gpui and of the document — so
/// a name from the page cannot reach it. What the command *does* still changes, and saying so
/// is what stops "group" reading as "make a new region" when it will grow an existing one.
fn group_label(scope: &Scope) -> &'static str {
    if scope.regions.can_fold {
        "Add selection to its region"
    } else {
        "Group selection into a region"
    }
}

fn regions_label(scope: &Scope) -> &'static str {
    if scope.settings.regions_enabled {
        "Disable regions"
    } else {
        "Enable regions"
    }
}

fn can_group(scope: &Scope) -> bool {
    scope.regions.can_group
}

fn can_ungroup(scope: &Scope) -> bool {
    scope.regions.can_ungroup
}

/// The picker is the only way to reach a region by name, so it is offered whenever the feature
/// is on — including with no regions yet, where it says how to make one.
fn regions_enabled(scope: &Scope) -> bool {
    scope.settings.regions_enabled
}

/// Both AI groupings run through the local model, so they are hidden when `ai.ollama` is not
/// configured — the reference hides them for the same reason. What each would have to work
/// with is [`grouping`]'s rule, asked here with the counters `Scope` carries rather than
/// restated.
fn can_group_with_ai(scope: &Scope) -> bool {
    regions_enabled(scope)
        && scope.ai.local_model
        && grouping::can_extend(scope.regions.ungrouped, scope.regions.count)
}

fn can_regroup_all_with_ai(scope: &Scope) -> bool {
    regions_enabled(scope)
        && scope.ai.local_model
        && grouping::can_partition(scope.regions.groupable)
}

pub(super) static ENTRIES: &[Command] = &[
    Command {
        id: "Region::GroupSelection",
        title: "Group selection into a region",
        label: Some(group_label),
        group: Group::Region,
        keywords: "group region cluster area section label waypoint wayfinding merge",
        default_keys: &["meta-g"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::region::GroupSelection),
        available: can_group,
    },
    Command {
        id: "Region::UngroupSelection",
        title: "Ungroup selection from its region",
        label: None,
        group: Group::Region,
        keywords: "ungroup region remove split wayfinding",
        default_keys: &["meta-shift-g"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::region::UngroupSelection),
        available: can_ungroup,
    },
    Command {
        id: "Region::OpenPicker",
        title: "Open the regions picker",
        label: None,
        group: Group::Region,
        keywords: "regions list menu waypoints wayfinding beacons go to",
        default_keys: &["r"],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::region::OpenPicker),
        available: regions_enabled,
    },
    Command {
        id: "Region::GroupWithAi",
        title: "Group ungrouped nodes with AI",
        label: None,
        group: Group::Region,
        keywords: "group ai regions cluster organize wayfinding suggest ungrouped",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::region::GroupWithAi),
        available: can_group_with_ai,
    },
    Command {
        id: "Region::RegroupAllWithAi",
        title: "Regroup all nodes with AI",
        label: None,
        group: Group::Region,
        keywords: "regroup reorganize ai regions cluster reshape wayfinding all",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::region::RegroupAllWithAi),
        available: can_regroup_all_with_ai,
    },
    Command {
        id: "Settings::ToggleRegions",
        // The stable name, for the keymap modal and tooltips; the palette shows `label`.
        title: "Enable or disable regions",
        label: Some(regions_label),
        group: Group::Region,
        keywords: "regions waypoints wayfinding beacons canvas toggle setting enable disable",
        default_keys: &[],
        context: CANVAS_NOT_TYPING,
        build: || Box::new(actions::settings::ToggleRegions),
        available: always,
    },
];
