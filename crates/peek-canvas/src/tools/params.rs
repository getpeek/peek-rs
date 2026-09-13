//! Reading a tool call's arguments.
//!
//! The MCP bridge serializes optional fields as an explicit `null` rather than omitting them, and
//! a local model may omit anything at all. Every accessor here filters on the *value's type*, so
//! "absent" and "present but null" collapse to `None` without a check at each call site.

use peek_document::geometry::{Point, Size};
use peek_document::{NodeId, PageId, RegionId, VariableRow, VariableValue, is_variable_name};
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub(super) struct Params<'a>(pub(super) &'a Value);

impl<'a> Params<'a> {
    pub(super) fn text(self, key: &str) -> Option<&'a str> {
        self.0.get(key)?.as_str()
    }

    pub(super) fn number(self, key: &str) -> Option<f64> {
        self.0.get(key)?.as_f64()
    }

    /// A non-negative whole number, for the one argument that is an index rather than a
    /// measurement. Read as an integer so it never round-trips through `f64`.
    pub(super) fn count(self, key: &str) -> Option<usize> {
        usize::try_from(self.0.get(key)?.as_u64()?).ok()
    }

    pub(super) fn flag(self, key: &str) -> Option<bool> {
        self.0.get(key)?.as_bool()
    }

    pub(super) fn node(self, key: &str) -> Option<NodeId> {
        self.text(key).map(NodeId::from)
    }

    pub(super) fn page(self, key: &str) -> Option<PageId> {
        self.text(key).map(PageId::from)
    }

    pub(super) fn region(self, key: &str) -> Option<RegionId> {
        self.text(key).map(RegionId::from)
    }

    /// `[x, y]`. A pair that is not two numbers is treated as absent rather than as zero, so a
    /// malformed position falls back to the caller's default instead of stacking at the origin.
    pub(super) fn point(self, key: &str) -> Option<Point> {
        let pair = self.pair(key)?;
        Some(Point::new(pair.0, pair.1))
    }

    /// `[width, height]`.
    pub(super) fn size(self, key: &str) -> Option<Size> {
        let pair = self.pair(key)?;
        Some(Size::new(pair.0, pair.1))
    }

    fn pair(self, key: &str) -> Option<(f64, f64)> {
        let array = self.0.get(key)?.as_array()?;
        let [first, second] = array.as_slice() else {
            return None;
        };
        Some((first.as_f64()?, second.as_f64()?))
    }

    /// Node ids, de-duplicated but left in the order the agent wrote them. Absent, null and an
    /// empty array are all the empty list — `select_nodes` gives that a meaning of its own.
    pub(super) fn nodes(self, key: &str) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = Vec::new();
        let Some(array) = self.0.get(key).and_then(Value::as_array) else {
            return ids;
        };
        for id in array.iter().filter_map(Value::as_str).map(NodeId::from) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// A `{ name: value }` map, where a value is a string or a list of strings.
    ///
    /// Insertion order is preserved (`serde_json` is built with `preserve_order`), so the rows
    /// land in the order the agent wrote them rather than alphabetically.
    pub(super) fn variables(self, key: &str) -> Option<Result<Vec<VariableRow>, String>> {
        let map = self.0.get(key)?.as_object()?;
        if map.is_empty() {
            return Some(Err("variables map is empty".to_string()));
        }
        let mut rows = Vec::with_capacity(map.len());
        for (name, value) in map {
            if !is_variable_name(name) {
                return Some(Err(format!("invalid variable name: {name}")));
            }
            rows.push(VariableRow {
                name: name.clone(),
                value: variable_value(value),
            });
        }
        Some(Ok(rows))
    }
}

fn variable_value(value: &Value) -> VariableValue {
    match value {
        Value::Array(items) => VariableValue::Many(items.iter().map(scalar).collect()),
        other => VariableValue::One(scalar(other)),
    }
}

/// Numbers and booleans are substituted into SQL as they were written, so they round-trip as
/// text rather than through `to_string`'s JSON quoting.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The error a missing required argument reports. Required-ness is enforced by the MCP schema
/// but not by a local model, so every required read goes through this.
pub(super) fn required<T>(value: Option<T>, key: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("missing or invalid '{key}'"))
}
