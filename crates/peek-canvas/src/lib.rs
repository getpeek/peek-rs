//! Everything about the canvas that needs no window: camera maths, camera flights, the
//! gesture state machine, hit-testing, grid spacing and the session document
//! (selection + revision) that views and later MCP/multiplayer mutate.

pub mod agent;
pub mod camera;
pub mod direction;
pub mod edge;
pub mod execution;
pub mod flight;
pub mod gesture;
pub mod grid;
mod history;
pub mod hit;
pub mod jump;
mod model;
pub mod regions;
mod scope;
pub mod stroke;
pub mod tools;

pub use camera::{Camera, MAX_ZOOM, MIN_ZOOM};
pub use direction::Direction;
pub use flight::{CameraFlight, Easing};
pub use jump::{JumpMode, JumpTarget, Pressed};
pub use model::{DRAW_COLOR, DRAW_STROKE_WIDTH, Document};
pub use peek_document::geometry::{Point, Rect, Size};
pub use regions::NewRegion;
pub use scope::{HistoryScope, Scope};
pub use tools::{CameraMove, ToolCall, ToolOutcome, execute};
