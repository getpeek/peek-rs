use crate::geometry::Size;

/// Size used when a node has neither `measured` nor `width`/`height` (`nodeGeometry.ts`).
pub const FALLBACK_SIZE: f64 = 200.0;

/// The node kinds the TypeScript app knows, by their on-disk `type` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeType {
    Query,
    Result,
    ResultInsertForm,
    Agent,
    Barchart,
    QueryError,
    TableDefinition,
    Text,
    Variable,
    Draw,
    Activity,
}

impl NodeType {
    pub const ALL: [Self; 11] = [
        Self::Query,
        Self::Result,
        Self::ResultInsertForm,
        Self::Agent,
        Self::Barchart,
        Self::QueryError,
        Self::TableDefinition,
        Self::Text,
        Self::Variable,
        Self::Draw,
        Self::Activity,
    ];

    /// The `type` tag as written to disk.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Result => "result",
            Self::ResultInsertForm => "result-insert-form",
            Self::Agent => "agent",
            Self::Barchart => "barchart",
            Self::QueryError => "query-error",
            Self::TableDefinition => "table-definition",
            Self::Text => "text",
            Self::Variable => "variable",
            Self::Draw => "draw",
            Self::Activity => "activity",
        }
    }

    /// Uppercase header label (`NodeIndicator.tsx`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Query => "QUERY",
            Self::Result => "RESULT",
            Self::ResultInsertForm => "INSERT",
            Self::Agent => "AGENT",
            Self::Barchart => "CHART",
            Self::QueryError => "ERROR",
            Self::TableDefinition => "TABLE",
            Self::Text => "TEXT",
            Self::Variable => "VARS",
            Self::Draw => "DRAW",
            Self::Activity => "ACTIVITY",
        }
    }

    /// Size a freshly placed node gets (`defaults.ts`).
    #[must_use]
    pub const fn default_size(self) -> Size {
        match self {
            Self::Query => Size::new(350.0, 240.0),
            Self::Result => Size::new(600.0, 440.0),
            Self::ResultInsertForm => Size::new(560.0, 220.0),
            Self::Agent => Size::new(540.0, 400.0),
            Self::Barchart => Size::new(460.0, 290.0),
            Self::QueryError => Size::new(400.0, 300.0),
            Self::TableDefinition => Size::new(450.0, 280.0),
            Self::Text => Size::new(280.0, 140.0),
            Self::Variable => Size::new(280.0, 220.0),
            Self::Draw => Size::new(100.0, 100.0),
            Self::Activity => Size::new(960.0, 520.0),
        }
    }

    /// Smallest size the resize handles allow (`defaults.ts`).
    #[must_use]
    pub const fn min_size(self) -> Size {
        match self {
            Self::Query => Size::new(320.0, 200.0),
            Self::Result => Size::new(400.0, 260.0),
            Self::ResultInsertForm => Size::new(360.0, 160.0),
            Self::Agent => Size::new(400.0, 300.0),
            Self::Barchart | Self::QueryError => Size::new(300.0, 200.0),
            Self::TableDefinition => Size::new(300.0, 140.0),
            Self::Text => Size::new(80.0, 32.0),
            Self::Variable => Size::new(220.0, 140.0),
            Self::Draw => Size::new(1.0, 1.0),
            Self::Activity => Size::new(620.0, 320.0),
        }
    }
}
