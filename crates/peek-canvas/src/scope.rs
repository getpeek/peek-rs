/// What the command registry needs to decide availability, snapshotted from the session
/// document. A handful of counters, so recomputing it per frame costs nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scope {
    pub selected: usize,
    pub selected_edges: usize,
    pub selected_queries: usize,
    pub selected_results: usize,
    pub selected_agents: usize,
    /// Every query node on the active page, selected or not: "rerun all" needs to know
    /// whether there is anything to rerun.
    pub queries: usize,
    pub pages: usize,
    pub history: HistoryScope,
    pub regions: RegionScope,
    pub ai: AiScope,
    pub camera_locked: bool,
    pub chrome_hidden: bool,
    pub connected: bool,
    /// Settings a command's *label* depends on, so a toggle can name what pressing it does
    /// rather than what it controls. `Command::label` is handed only a `Scope`, which is what
    /// keeps the registry free of gpui — the price is that a setting a label reads has to be
    /// projected here by `CanvasView::scope`.
    pub settings: SettingsScope,
}

/// What the region commands need to know, kept together like [`HistoryScope`] so each of them
/// reads one thing rather than three loose counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionScope {
    /// Regions on the active page with at least one live member.
    pub count: usize,
    /// The selection would create or grow a region — [`crate::GroupPlan`] says which.
    pub can_group: bool,
    /// Grouping would grow an existing region rather than mint one, so the command can say so.
    pub can_fold: bool,
    /// Some selected node sits in a region, so there is something to pull out.
    pub can_ungroup: bool,
    /// Nodes on the active page that no region holds, drawings excluded — what "group
    /// ungrouped with AI" would have to work with.
    pub ungrouped: usize,
    /// Nodes a grouping may touch at all, which is every node but the drawings.
    pub groupable: usize,
}

/// What `settings.json` says about the local model. The AI commands are offered only when one
/// is configured, and a toggle names what pressing it does — both of which the registry has to
/// know without reading the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiScope {
    /// `ai.ollama` is set, so there is something local to ask.
    pub local_model: bool,
    /// `ai.automatically_label_queries`.
    pub labels_queries: bool,
}

/// The slice of `settings.json` the palette's labels read. Kept in its own struct, like
/// [`HistoryScope`], so the next toggle adds a field here rather than another loose bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsScope {
    pub pages_as_list: bool,
    pub palette_button_hidden: bool,
    pub regions_enabled: bool,
}

/// Undo availability for the active page, kept together so commands read one thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryScope {
    pub can_undo: bool,
    pub can_redo: bool,
}
