use serde::{Deserialize, Serialize};

use crate::ids::{NodeId, RegionId};

/// Entries in a theme's region palette. `color_index` is assigned modulo this, so a document
/// written by one theme stays legible under another.
pub const REGION_COLOR_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegionStatus {
    Confirmed,
    /// An AI proposal awaiting review. Renaming implicitly confirms.
    Suggested,
}

/// A named, coloured, position-less set of node ids; its geometry is derived from the
/// members' bounding box at render time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub id: RegionId,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    /// Index into the theme's region palette (`--pk-region-N`), so regions recolour per theme.
    pub color_index: u8,
    pub status: RegionStatus,
    /// Members may since have been deleted — filter against live nodes when deriving.
    pub member_ids: Vec<NodeId>,
}
