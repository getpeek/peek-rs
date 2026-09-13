/// What the command registry needs to decide availability, snapshotted from the session
/// document. A handful of counters, so recomputing it per frame costs nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scope {
    pub selected: usize,
    pub selected_edges: usize,
    pub selected_queries: usize,
    pub selected_results: usize,
    pub selected_agents: usize,
    pub pages: usize,
    pub history: HistoryScope,
    pub camera_locked: bool,
    pub chrome_hidden: bool,
    pub connected: bool,
}

/// Undo availability for the active page, kept together so commands read one thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryScope {
    pub can_undo: bool,
    pub can_redo: bool,
}
