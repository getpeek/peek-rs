//! The canvas document as persisted at `~/peek/workspaces/<workspace>/<connection>.json`.
//!
//! The on-disk shape is owned by the TypeScript app (`~/labs/peek/src/canvas/types.ts`) and is
//! frozen: everything here must read what it writes and write what it reads. React Flow's
//! runtime fields (`selected`, `dragging`, `resizing`, `measured`, `className`, `style`) are
//! tolerated on read and dropped on write.

mod agent;
mod document;
mod edge;
pub mod export;
pub mod geometry;
pub mod history;
mod ids;
mod kinds;
mod migrate;
mod node;
mod region;
pub mod result;
mod results_file;
mod sql_type;
mod storage;

pub use agent::{AgentMessage, PlanEntry, ToolCall};
pub use document::{CanvasDocument, DocVersion, DocumentError, Page, Viewport};
pub use edge::Edge;
pub use export::{filename as export_filename, slug as export_slug, to_csv, to_json};
pub use ids::{CheckpointId, EdgeId, NodeId, PageId, RegionId};
pub use kinds::{FALLBACK_SIZE, NodeType};
pub use migrate::normalize;
pub use node::{
    ActivityData, ActivityFilter, AgentData, AgentProvider, BarChartData, ChartType, DrawData,
    ErrorData, LiveInterval, Node, NodeData, NodeKind, QueryData, ResultData, ResultInsertFormData,
    TableDefinitionData, TextData, VariableData, VariableRow, VariableValue, is_variable_name,
};
pub use region::{REGION_COLOR_COUNT, Region, RegionStatus};
pub use result::{Cell, Column, ResultSet};
pub use results_file::{ResultSidecar, ResultsFile};
pub use sql_type::{
    MIN_RESULT_WIDTH, is_boolean, is_json, is_numeric, is_text, is_timestamp, is_uuid,
    placement_column_width,
};
pub use storage::{DocumentFile, DocumentStore, StorageError};
