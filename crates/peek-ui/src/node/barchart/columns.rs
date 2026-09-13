//! Which column is the category axis and which columns are the numeric series.
//!
//! Mirrors `BarChartNode.tsx`: both questions are answered from the *first* row alone, the
//! axis is the first string-valued column, and the series are every numeric column except
//! `id` and anything ending in `_id`. "First" is the query's own column order, which
//! [`peek_document::BarChartData`] preserves for exactly this reason.

use peek_document::BarChartData;

/// `BarChartNode.tsx`'s axis fallback when no column holds a string.
const FALLBACK_AXIS: &str = "name";
/// ... and its series fallback when no column holds a number.
const FALLBACK_SERIES: &str = "value";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChartColumns {
    pub(crate) axis: String,
    pub(crate) series: Vec<String>,
}

impl ChartColumns {
    pub(crate) fn of(data: &BarChartData) -> Self {
        let Some(first) = data.data.first() else {
            return Self {
                axis: FALLBACK_AXIS.to_string(),
                series: Vec::new(),
            };
        };
        Self {
            axis: first
                .iter()
                .find(|(_, value)| value.is_string())
                .map_or_else(|| FALLBACK_AXIS.to_string(), |(key, _)| key.clone()),
            series: first
                .iter()
                .filter(|(key, value)| value.is_number() && is_measure(key))
                .map(|(key, _)| key.clone())
                .collect(),
        }
    }

    /// The name the header and the body title carry: the first series, or the TSX fallback.
    pub(crate) fn primary_series(&self) -> &str {
        self.series.first().map_or(FALLBACK_SERIES, String::as_str)
    }

    /// One label per row. A row without the axis column, or with a non-string in it, has no
    /// name to show and contributes an empty band.
    pub(crate) fn labels(&self, data: &BarChartData) -> Vec<String> {
        data.data
            .iter()
            .map(|row| {
                row.get(&self.axis)
                    .and_then(|value| Some(value.as_str()?.to_string()))
                    .unwrap_or_default()
            })
            .collect()
    }
}

/// One series' value per row; `None` where the row has no finite number under `key`, which
/// leaves a gap rather than a zero.
pub(crate) fn values(data: &BarChartData, key: &str) -> Vec<Option<f64>> {
    data.data
        .iter()
        .map(|row| {
            row.get(key)
                .and_then(|value| value.as_f64().filter(|number| number.is_finite()))
        })
        .collect()
}

/// `id` and `*_id` are join keys: numeric, but never a magnitude worth plotting.
fn is_measure(key: &str) -> bool {
    key != "id" && !key.ends_with("_id")
}

/// Test data built through the real on-disk shape, so the tests that use it exercise the
/// same deserialization path the canvas does.
#[cfg(test)]
pub(super) fn chart(rows: &str) -> BarChartData {
    use peek_document::{CanvasDocument, NodeData};

    let json = format!(
        r#"{{"version":1,"activePageId":"p","pageOrder":["p"],"pages":{{"p":{{
          "id":"p","name":"P","edges":[],"viewport":{{"x":0,"y":0,"zoom":1}},
          "nodes":[{{"id":"barchart_0001","type":"barchart",
            "position":{{"x":0,"y":0}},"data":{{"data":{rows}}}}}]}}}}}}"#
    );
    let document = CanvasDocument::from_json(&json).expect("fixture parses");
    let node = &document.active_page().expect("page").nodes[0];
    BarChartData::get(&node.kind)
        .expect("barchart data")
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axis_is_the_first_string_column_and_numbers_are_series() {
        let columns = ChartColumns::of(&chart(r#"[{"name":"Peek","count":22}]"#));
        assert_eq!(columns.axis, "name");
        assert_eq!(columns.series, vec!["count".to_string()]);
        assert_eq!(columns.primary_series(), "count");
    }

    #[test]
    fn id_and_suffixed_id_columns_are_not_series() {
        let columns = ChartColumns::of(&chart(
            r#"[{"id":7,"customer_id":3,"customer_name":"Vicosight","total_quotes":142}]"#,
        ));
        assert_eq!(columns.axis, "customer_name");
        assert_eq!(columns.series, vec!["total_quotes".to_string()]);
    }

    #[test]
    fn every_remaining_numeric_column_becomes_a_series() {
        let columns = ChartColumns::of(&chart(
            r#"[{"customer_name":"Vicosight","total_quotes":142,"signed_quotes":12}]"#,
        ));
        assert_eq!(columns.axis, "customer_name");
        assert_eq!(
            columns.series,
            vec!["total_quotes".to_string(), "signed_quotes".to_string()],
            "the series follow the query's column order, not an alphabetical one"
        );
    }

    #[test]
    fn a_row_without_a_string_or_a_number_falls_back() {
        let columns = ChartColumns::of(&chart(r#"[{"ok":true,"id":4}]"#));
        assert_eq!(columns.axis, "name");
        assert!(columns.series.is_empty());
        assert_eq!(columns.primary_series(), "value");
    }

    #[test]
    fn no_rows_means_no_columns() {
        let columns = ChartColumns::of(&chart("[]"));
        assert_eq!(columns.axis, "name");
        assert!(columns.series.is_empty());
    }

    #[test]
    fn later_rows_may_be_missing_the_columns_the_first_row_declared() {
        let data =
            chart(r#"[{"name":"a","count":1},{"count":2},{"name":"c"},{"name":"d","count":null}]"#);
        let columns = ChartColumns::of(&data);
        assert_eq!(
            columns.labels(&data),
            vec![
                "a".to_string(),
                String::new(),
                "c".to_string(),
                "d".to_string()
            ]
        );
        assert_eq!(
            values(&data, "count"),
            vec![Some(1.0), Some(2.0), None, None]
        );
    }

    #[test]
    fn the_classification_matches_the_real_workspace_fixture() {
        // The four barchart nodes in `peek-document/tests/fixtures/plock-local.json`.
        let cases = [
            (
                r#"[{"quote_status":"Vicosight draft","cnt":160}]"#,
                "quote_status",
                vec!["cnt"],
            ),
            (r#"[{"name":"Peek","count":22}]"#, "name", vec!["count"]),
            (
                r#"[{"customer_name":"Vicosight","total_quotes":142,"signed_quotes":12}]"#,
                "customer_name",
                vec!["total_quotes", "signed_quotes"],
            ),
            (
                r#"[{"customer_name":"Learnster","total_quotes":1,"signed_quotes":0}]"#,
                "customer_name",
                vec!["total_quotes", "signed_quotes"],
            ),
        ];
        for (rows, axis, series) in cases {
            let columns = ChartColumns::of(&chart(rows));
            assert_eq!(columns.axis, axis);
            assert_eq!(columns.series, series);
        }
    }
}
