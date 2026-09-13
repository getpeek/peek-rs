use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::agent::AgentMessage;
use crate::geometry::{Point, Rect, Size};
use crate::ids::NodeId;
use crate::kinds::{FALLBACK_SIZE, NodeType};

/// A React Flow node as Peek persists it. Only the fields Peek authors are declared; React
/// Flow's runtime extras (`dragging`, `resizing`, `className`, `style`) fall into the
/// flattened `kind` and are discarded, so a clean write drops them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: NodeId,
    pub position: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// React Flow's last DOM measurement. Read for the size fallback, never written: this
    /// app is the measurer now.
    #[serde(default, skip_serializing)]
    pub measured: Option<Size>,
    /// Seeds the session selection on load; never written.
    #[serde(default, skip_serializing)]
    pub selected: bool,
    #[serde(flatten)]
    pub kind: NodeKind,
}

impl Node {
    /// `measured ?? width/height ?? 200`, per axis, exactly as `nodeGeometry.ts` resolves it.
    #[must_use]
    pub fn size(&self) -> Size {
        let measured = self.measured.unwrap_or_default();
        let width = self
            .measured
            .map(|_| measured.width)
            .or(self.width)
            .unwrap_or(FALLBACK_SIZE);
        let height = self
            .measured
            .map(|_| measured.height)
            .or(self.height)
            .unwrap_or(FALLBACK_SIZE);
        Size::new(width, height)
    }

    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(self.position, self.size())
    }

    #[must_use]
    pub fn node_type(&self) -> Option<NodeType> {
        self.kind.node_type()
    }

    /// A freshly created node, as `defaults.ts` `makeNode` builds one: a minted id, empty
    /// per-kind data, and the caller's geometry clamped to the kind's minimum size.
    #[must_use]
    pub fn new(node_type: NodeType, bounds: Rect) -> Self {
        let minimum = node_type.min_size();
        Self {
            id: NodeId::for_type(node_type),
            position: bounds.origin,
            width: Some(bounds.size.width.max(minimum.width)),
            height: Some(bounds.size.height.max(minimum.height)),
            measured: None,
            selected: false,
            kind: NodeKind::empty(node_type),
        }
    }
}

/// Lets a caller name a kind's payload by type rather than by matching on [`NodeKind`], so
/// node views and the mutation API stay closed over the eleven kinds.
pub trait NodeData: Sized {
    fn get(kind: &NodeKind) -> Option<&Self>;
    fn get_mut(kind: &mut NodeKind) -> Option<&mut Self>;
}

macro_rules! node_data {
    ($($variant:ident => $data:ty),+ $(,)?) => {
        $(
            impl NodeData for $data {
                fn get(kind: &NodeKind) -> Option<&Self> {
                    match kind {
                        NodeKind::$variant(data) => Some(data),
                        _ => None,
                    }
                }

                fn get_mut(kind: &mut NodeKind) -> Option<&mut Self> {
                    match kind {
                        NodeKind::$variant(data) => Some(data),
                        _ => None,
                    }
                }
            }
        )+
    };
}

node_data! {
    Query => QueryData,
    Result => ResultData,
    ResultInsertForm => ResultInsertFormData,
    Agent => AgentData,
    Barchart => BarChartData,
    QueryError => ErrorData,
    TableDefinition => TableDefinitionData,
    Text => TextData,
    Variable => VariableData,
    Draw => DrawData,
    Activity => ActivityData,
}

/// Per-kind payload, tagged by the sibling `type` field with its data under `data`.
///
/// Serialization is derived; deserialization is hand-rolled (below) because serde's
/// `#[serde(other)]` only accepts a bare tag, and an unknown kind still carries `data`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "kebab-case")]
pub enum NodeKind {
    Query(QueryData),
    Result(ResultData),
    ResultInsertForm(ResultInsertFormData),
    Agent(AgentData),
    Barchart(BarChartData),
    QueryError(ErrorData),
    TableDefinition(TableDefinitionData),
    Text(TextData),
    Variable(VariableData),
    Draw(DrawData),
    Activity(ActivityData),
    /// A `type` this build doesn't know. Tolerated on read, dropped by [`crate::normalize`].
    Unknown,
}

impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Tagged {
            #[serde(rename = "type")]
            tag: String,
            #[serde(default)]
            data: Value,
        }

        fn parse<T: serde::de::DeserializeOwned, E: Error>(data: Value) -> Result<T, E> {
            serde_json::from_value(data).map_err(E::custom)
        }

        let Tagged { tag, data } = Tagged::deserialize(deserializer)?;
        Ok(match tag.as_str() {
            "query" => Self::Query(parse(data)?),
            "result" => Self::Result(parse(data)?),
            "result-insert-form" => Self::ResultInsertForm(parse(data)?),
            "agent" => Self::Agent(parse(data)?),
            "barchart" => Self::Barchart(parse(data)?),
            "query-error" => Self::QueryError(parse(data)?),
            "table-definition" => Self::TableDefinition(parse(data)?),
            "text" => Self::Text(parse(data)?),
            "variable" => Self::Variable(parse(data)?),
            "draw" => Self::Draw(parse(data)?),
            "activity" => Self::Activity(parse(data)?),
            _ => Self::Unknown,
        })
    }
}

impl NodeKind {
    /// The per-kind `data` a new node starts with, transcribed from `defaults.ts` `makeNode`.
    #[must_use]
    pub fn empty(node_type: NodeType) -> Self {
        match node_type {
            NodeType::Query => Self::Query(QueryData::default()),
            NodeType::Result => Self::Result(ResultData::default()),
            NodeType::ResultInsertForm => Self::ResultInsertForm(ResultInsertFormData {
                result_node_id: NodeId::new(String::new()),
                initial_values: None,
            }),
            NodeType::Agent => Self::Agent(AgentData::default()),
            NodeType::Barchart => Self::Barchart(BarChartData {
                data: Vec::new(),
                chart_type: Some(ChartType::Bar),
            }),
            NodeType::QueryError => Self::QueryError(ErrorData {
                query_node_id: NodeId::new(String::new()),
                query: String::new(),
                message: String::new(),
            }),
            NodeType::TableDefinition => Self::TableDefinition(TableDefinitionData::default()),
            NodeType::Text => Self::Text(TextData::default()),
            NodeType::Variable => Self::Variable(VariableData {
                rows: vec![VariableRow {
                    name: String::new(),
                    value: VariableValue::One(String::new()),
                }],
                is_global: None,
            }),
            NodeType::Draw => Self::Draw(DrawData {
                points: Vec::new(),
                stroke_width: 4.0,
                color: "white".to_string(),
            }),
            NodeType::Activity => Self::Activity(ActivityData {
                filter: ActivityFilter::All,
                live: true,
                min_secs: 0.0,
            }),
        }
    }

    #[must_use]
    pub fn node_type(&self) -> Option<NodeType> {
        Some(match self {
            Self::Query(_) => NodeType::Query,
            Self::Result(_) => NodeType::Result,
            Self::ResultInsertForm(_) => NodeType::ResultInsertForm,
            Self::Agent(_) => NodeType::Agent,
            Self::Barchart(_) => NodeType::Barchart,
            Self::QueryError(_) => NodeType::QueryError,
            Self::TableDefinition(_) => NodeType::TableDefinition,
            Self::Text(_) => NodeType::Text,
            Self::Variable(_) => NodeType::Variable,
            Self::Draw(_) => NodeType::Draw,
            Self::Activity(_) => NodeType::Activity,
            Self::Unknown => return None,
        })
    }
}

/// `liveIntervalMs` on disk: a number while live polling runs, `null` once it has been
/// switched off, absent if it never was. `null` and absent must stay distinguishable so a
/// clean write reproduces the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Option<u64>", into = "Option<u64>")]
pub enum LiveInterval {
    Off,
    EveryMs(u64),
}

impl From<Option<u64>> for LiveInterval {
    fn from(value: Option<u64>) -> Self {
        value.map_or(Self::Off, Self::EveryMs)
    }
}

impl From<LiveInterval> for Option<u64> {
    fn from(value: LiveInterval) -> Self {
        match value {
            LiveInterval::Off => None,
            LiveInterval::EveryMs(ms) => Some(ms),
        }
    }
}

/// Deserializes a present-but-possibly-`null` key as `Some`, bypassing `Option`'s own
/// null handling.
fn present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryData {
    pub query: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub live_interval_ms: Option<LiveInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultData {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_widths: Option<BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivoted: Option<bool>,
    /// Node size before pivoting, restored when toggling pivot back off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_pivot_size: Option<Size>,
    /// Pre-sidecar documents inlined the rows here. Read so the results milestone can lift
    /// them into the sidecar the way `useLoadDocument` does; never written back.
    #[serde(rename = "data", default, skip_serializing)]
    pub legacy_rows: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultInsertFormData {
    pub result_node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_values: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProvider {
    Ollama,
    Acp,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentData {
    /// Vestigial: `makeNode` writes `""` and nothing reads it, but all real documents carry
    /// it, so it stays on the wire.
    pub query: String,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartType {
    Bar,
    Line,
    Area,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarChartData {
    /// Rows keep the query's column order, so this is `serde_json::Map` (an `IndexMap` under
    /// the `preserve_order` feature) rather than a `BTreeMap`, which would sort the keys.
    /// The order is load-bearing: `BarChartNode.tsx` takes its axis from the first
    /// string-valued column and its primary series from the first numeric one, so writing the
    /// columns back sorted would silently re-label a chart the TypeScript app then reopens.
    #[serde(default)]
    pub data: Vec<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<ChartType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorData {
    pub query_node_id: NodeId,
    pub query: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableDefinitionData {
    pub table: String,
    pub columns: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextData {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VariableValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableRow {
    pub name: String,
    pub value: VariableValue,
}

/// Whether a name can actually be substituted into a query.
///
/// `VARIABLE_NAME_RE` in `~/labs/peek/src/canvas/variables.ts` is `/^[A-Za-z_][A-Za-z0-9_]*$/u`
/// and `scanVariableSites` only ever matches that shape, so a name outside it is inert however
/// valid it looks. The grammar is six characters wide, which is cheaper to spell out than to
/// pull a regex engine in for.
#[must_use]
pub fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableData {
    #[serde(default)]
    pub rows: Vec<VariableRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_global: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawData {
    /// `[x, y, pressure]` triples relative to the node origin.
    #[serde(default)]
    pub points: Vec<[f64; 3]>,
    pub stroke_width: f64,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityFilter {
    All,
    Active,
    IdleInTxn,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityData {
    pub filter: ActivityFilter,
    pub live: bool,
    /// Backends younger than this are hidden, so the list isn't flooded by short queries.
    pub min_secs: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_prefers_measured_then_explicit_then_fallback() {
        let mut node: Node = serde_json::from_str(
            r#"{"id":"text_1","type":"text","position":{"x":1,"y":2},"width":300,"height":100,
                "measured":{"width":310,"height":120},"data":{"text":"hi"}}"#,
        )
        .unwrap();
        assert_eq!(node.size(), Size::new(310.0, 120.0));
        node.measured = None;
        assert_eq!(node.size(), Size::new(300.0, 100.0));
        node.width = None;
        node.height = None;
        assert_eq!(node.size(), Size::new(FALLBACK_SIZE, FALLBACK_SIZE));
    }

    #[test]
    fn ephemeral_fields_are_read_and_dropped() {
        let raw = r#"{"id":"query_1","type":"query","position":{"x":0,"y":0},"width":350,"height":240,
            "selected":true,"dragging":false,"resizing":false,"className":"x","style":{"pointerEvents":"none"},
            "measured":{"width":350,"height":240},"data":{"query":"select 1","isRunning":true}}"#;
        let node: Node = serde_json::from_str(raw).unwrap();
        assert!(node.selected);
        assert_eq!(node.node_type(), Some(NodeType::Query));

        let written = serde_json::to_value(&node).unwrap();
        let mut keys: Vec<&String> = written.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["data", "height", "id", "position", "type", "width"]);
        assert_eq!(written["type"], "query");
        assert_eq!(written["data"]["isRunning"], true);
    }

    /// Column order is load-bearing and easy to lose: it is only preserved because
    /// `serde_json`'s `preserve_order` feature is on (transitively, via gpui). If that ever
    /// stops being true this test fails rather than quietly re-labelling users' charts.
    #[test]
    fn chart_rows_keep_the_querys_column_order() {
        let json = r#"{
            "id": "query_abc12345-chart",
            "type": "barchart",
            "position": { "x": 0, "y": 0 },
            "data": { "data": [{ "customer_name": "V", "total_quotes": 142, "signed_quotes": 12 }] }
        }"#;
        let node: Node = serde_json::from_str(json).unwrap();

        let data = BarChartData::get(&node.kind).unwrap();
        let columns: Vec<&str> = data.data[0].keys().map(String::as_str).collect();
        assert_eq!(
            columns,
            ["customer_name", "total_quotes", "signed_quotes"],
            "sorted keys would make signed_quotes the primary series"
        );

        let written = serde_json::to_string(&node).unwrap();
        let row = written.find("\"customer_name\"").unwrap();
        let total = written.find("\"total_quotes\"").unwrap();
        let signed = written.find("\"signed_quotes\"").unwrap();
        assert!(
            row < total && total < signed,
            "written back out of order: {written}"
        );
    }

    #[test]
    fn node_data_accessors_match_their_kind() {
        let mut text = NodeKind::empty(NodeType::Text);
        assert!(TextData::get(&text).is_some());
        assert!(QueryData::get(&text).is_none());
        TextData::get_mut(&mut text).unwrap().text = "hi".to_string();
        assert_eq!(TextData::get(&text).unwrap().text, "hi");

        // Every kind's own accessor finds it, and a foreign one does not.
        for node_type in NodeType::ALL {
            let kind = NodeKind::empty(node_type);
            assert_eq!(kind.node_type(), Some(node_type), "{node_type:?}");
        }
    }

    #[test]
    fn empty_variable_starts_with_one_blank_row() {
        let kind = NodeKind::empty(NodeType::Variable);
        let data = VariableData::get(&kind).unwrap();
        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.rows[0].value, VariableValue::One(String::new()));
    }

    #[test]
    fn new_node_clamps_to_min_size() {
        let node = Node::new(
            NodeType::Query,
            Rect::new(Point::new(10.0, 20.0), Size::new(10.0, 10.0)),
        );
        assert_eq!(node.size(), NodeType::Query.min_size());
        assert_eq!(node.position, Point::new(10.0, 20.0));
        assert!(node.id.as_str().starts_with("query_"));
    }

    #[test]
    fn every_kind_parses_and_unknown_is_tolerated() {
        let samples = [
            r#"{"type":"query","data":{"query":"select 1"}}"#,
            r#"{"type":"result","data":{"query":"select 1","columnWidths":{"a":120},"pivoted":true,"data":[[["a",1,"INT4"]]]}}"#,
            r#"{"type":"result-insert-form","data":{"resultNodeId":"q-result-0"}}"#,
            r#"{"type":"agent","data":{"query":"hi","messages":[{"type":"user","message":"x","timestamp":1}],"provider":"acp"}}"#,
            r#"{"type":"barchart","data":{"data":[{"day":"mon","n":3}],"chartType":"area"}}"#,
            r#"{"type":"query-error","data":{"queryNodeId":"q","query":"select","message":"boom"}}"#,
            r#"{"type":"table-definition","data":{"table":"users","columns":[["id","uuid"]]}}"#,
            r#"{"type":"text","data":{"text":"note"}}"#,
            r#"{"type":"variable","data":{"rows":[{"name":"a","value":"1"},{"name":"b","value":["1","2"]}],"isGlobal":true}}"#,
            r##"{"type":"draw","data":{"points":[[0,0,0.5]],"strokeWidth":4,"color":"#fff"}}"##,
            r#"{"type":"activity","data":{"filter":"idle-in-txn","live":true,"minSecs":2}}"#,
            r#"{"type":"hologram","data":{"whatever":1}}"#,
        ];
        for (index, sample) in samples.iter().enumerate() {
            let raw = format!(
                r#"{{"id":"n{index}","position":{{"x":0,"y":0}},{}"#,
                &sample[1..]
            );
            let node: Node =
                serde_json::from_str(&raw).unwrap_or_else(|error| panic!("{sample}: {error}"));
            let expected = NodeType::ALL.get(index).copied();
            assert_eq!(node.node_type(), expected, "{sample}");
        }
    }
}
