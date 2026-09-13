//! The flattened connection list the picker's cursor and search walk.
//!
//! Ported from `WorkspaceList.tsx`. The reference's cursor indexes connections, not workspaces:
//! a workspace header is never a landing spot, it only expands when the cursor enters it. One
//! flat sequence is what makes that simple, and it is what the arrow keys move through.

use gpui_kit::SharedString;
use peek_config::{PeekConfig, UrlParts};

use crate::fuzzy::{MATCH_THRESHOLD, score};

/// One connection as the picker shows it.
///
/// It deliberately carries no URL. The rows only need to name a connection well enough to pick
/// it; the form reads the full [`peek_config::DatabaseConnection`] back out of the config when
/// it opens, so credentials never sit in the list being rendered every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) workspace: SharedString,
    pub(crate) name: SharedString,
    pub(crate) user: SharedString,
    pub(crate) host: SharedString,
    /// Searchable but never shown, as the reference has it: people look a connection up by the
    /// database it opens more often than they can name its host.
    pub(crate) database: SharedString,
    pub(crate) tint: Option<(u8, u8, u8)>,
    /// Drives the `SSH` badge.
    pub(crate) tunnelled: bool,
    /// Character offsets the query matched, for highlighting. Empty when not searching.
    pub(crate) name_match: Vec<usize>,
    pub(crate) origin_match: Vec<usize>,
}

impl Entry {
    /// `user@host`, the row's second line. `None` when the URL gave neither.
    pub(crate) fn origin(&self) -> Option<SharedString> {
        (!self.user.is_empty() && !self.host.is_empty())
            .then(|| SharedString::from(format!("{}@{}", self.user, self.host)))
    }
}

/// Every connection in the config, flattened in file order so the grouping follows it.
pub(crate) fn entries(config: &PeekConfig) -> Vec<Entry> {
    config
        .workspaces
        .iter()
        .flat_map(|workspace| {
            workspace.connections.iter().map(|connection| {
                let parts = connection.parts().unwrap_or_default();
                let UrlParts {
                    user,
                    host,
                    database,
                    ..
                } = parts;
                Entry {
                    workspace: SharedString::from(workspace.name.clone()),
                    name: SharedString::from(connection.name.clone()),
                    user: SharedString::from(user),
                    host: SharedString::from(host),
                    database: SharedString::from(database),
                    tint: connection.rgb(),
                    tunnelled: connection.ssh_tunnel.is_some(),
                    name_match: Vec::new(),
                    origin_match: Vec::new(),
                }
            })
        })
        .collect()
}

/// Narrows and re-orders `entries` by `query`, and records where it matched.
///
/// The reference's five keys: the workspace name, the connection name, the user, the host and
/// the database. Scoring the workspace name is what makes typing a workspace surface all of its
/// connections even though the cursor never lands on the workspace itself.
///
/// An empty query is not a search: everything comes back in its own order, unhighlighted.
pub(crate) fn filter(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let query = query.trim();
    if query.is_empty() {
        return entries;
    }

    let mut scored: Vec<(f64, Entry)> = entries
        .into_iter()
        .filter_map(|entry| rank(entry, query))
        .collect();
    // Descending, and stable, so equally good rows keep their file order.
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.into_iter().map(|(_, entry)| entry).collect()
}

/// Gathers rows under their workspace, keeping the order they arrive in.
///
/// A workspace takes the position of its best row, and its rows keep their own order, so the
/// sequence read top to bottom is exactly the sequence the cursor walks. That matters more than
/// it sounds: the reference groups for display but keeps a flat, score-ordered cursor, so its
/// arrow keys can jump around the panel. Here the best overall match is still the first row of
/// the first group, because `filter` has already sorted by score.
pub(crate) fn group(entries: Vec<Entry>) -> Vec<(SharedString, Vec<Entry>)> {
    let mut grouped: Vec<(SharedString, Vec<Entry>)> = Vec::new();
    for entry in entries {
        match grouped
            .iter_mut()
            .find(|(workspace, _)| workspace == &entry.workspace)
        {
            Some((_, rows)) => rows.push(entry),
            None => grouped.push((entry.workspace.clone(), vec![entry])),
        }
    }
    grouped
}

/// Scores one entry, or drops it. Ranking reads all five keys; highlighting only marks the two
/// the row actually draws, which is why the origin is scored again as the one string it is
/// rendered as.
fn rank(mut entry: Entry, query: &str) -> Option<(f64, Entry)> {
    let keys = [
        &entry.workspace,
        &entry.name,
        &entry.user,
        &entry.host,
        &entry.database,
    ];
    let best = keys
        .into_iter()
        .filter_map(|key| score(key, query))
        .map(|found| found.score)
        .fold(0.0_f64, f64::max);
    if best < MATCH_THRESHOLD {
        return None;
    }

    entry.name_match = score(&entry.name, query)
        .filter(|found| found.score >= MATCH_THRESHOLD)
        .map(|found| found.indices)
        .unwrap_or_default();
    entry.origin_match = entry
        .origin()
        .and_then(|origin| score(&origin, query))
        .filter(|found| found.score >= MATCH_THRESHOLD)
        .map(|found| found.indices)
        .unwrap_or_default();
    Some((best, entry))
}

#[cfg(test)]
mod tests {
    use super::{Entry, entries, filter};
    use peek_config::{DatabaseConnection, PeekConfig, Workspace};

    fn config() -> PeekConfig {
        let connection = |name: &str, url: &str| DatabaseConnection {
            name: name.to_string(),
            color: "#5584E8".to_string(),
            url: url.to_string(),
            ssh_tunnel: None,
        };
        // `PeekConfig` keeps `$schema` private, so this builds rather than struct-updates.
        let mut config = PeekConfig::default();
        config.workspaces = vec![
            Workspace {
                name: "Peek".to_string(),
                connections: vec![connection("local", "postgres://dbuser:pw@postgres/nesso")],
            },
            Workspace {
                name: "Plock".to_string(),
                connections: vec![
                    connection("local", "postgres://metered_user:pw@localhost/forge"),
                    connection("staging", "postgres://postgres_test:pw@meteredtest.rds/dev"),
                ],
            },
        ];
        config
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| format!("{}/{}", entry.workspace, entry.name))
            .collect()
    }

    #[test]
    fn every_connection_is_flattened_in_file_order() {
        assert_eq!(
            names(&entries(&config())),
            ["Peek/local", "Plock/local", "Plock/staging"]
        );
    }

    #[test]
    fn an_entry_reads_its_origin_off_the_url() {
        let flattened = entries(&config());
        assert_eq!(
            flattened[1].origin().map(|origin| origin.to_string()),
            Some("metered_user@localhost".to_string())
        );
        assert_eq!(flattened[1].database, "forge");
    }

    #[test]
    fn an_empty_query_keeps_everything_in_its_own_order() {
        let flattened = entries(&config());
        assert_eq!(names(&filter(flattened, "   ")).len(), 3);
    }

    /// The load-bearing one: the cursor never lands on a workspace, so matching a workspace name
    /// has to surface that workspace's connections instead.
    #[test]
    fn matching_a_workspace_name_surfaces_its_connections() {
        let found = filter(entries(&config()), "plock");
        assert_eq!(names(&found), ["Plock/local", "Plock/staging"]);
    }

    #[test]
    fn a_connection_matches_on_its_host_and_on_its_database() {
        assert_eq!(
            names(&filter(entries(&config()), "meteredtest")),
            ["Plock/staging"]
        );
        assert_eq!(names(&filter(entries(&config()), "forge")), ["Plock/local"]);
    }

    #[test]
    fn grouping_keeps_the_order_the_rows_arrive_in() {
        let grouped = super::group(entries(&config()));
        let shape: Vec<(String, usize)> = grouped
            .iter()
            .map(|(workspace, rows)| (workspace.to_string(), rows.len()))
            .collect();
        assert_eq!(shape, [("Peek".to_string(), 1), ("Plock".to_string(), 2)]);
    }

    /// The cursor walks the flattened groups, so a workspace has to take the position of its
    /// best row — otherwise the first row on screen is not the row Enter would pick.
    #[test]
    fn a_workspace_takes_the_position_of_its_best_row() {
        let grouped = super::group(filter(entries(&config()), "staging"));
        assert_eq!(
            grouped.first().map(|(name, _)| name.to_string()),
            Some("Plock".to_string())
        );
        assert_eq!(grouped.len(), 1, "Peek had no match at all");
    }

    #[test]
    fn a_query_that_matches_nothing_leaves_no_rows() {
        assert!(filter(entries(&config()), "zzzzz").is_empty());
    }

    #[test]
    fn matched_characters_are_recorded_for_the_two_lines_the_row_draws() {
        let found = filter(entries(&config()), "staging");
        let entry = found.first().expect("staging matched");
        assert_eq!(entry.name_match, [0, 1, 2, 3, 4, 5, 6]);
        assert!(
            entry.origin_match.is_empty(),
            "the origin does not contain it"
        );
    }

    /// A workspace-name match must not underline characters in a connection name that never
    /// matched — the highlight has to mean the same thing as the ranking.
    #[test]
    fn a_workspace_match_leaves_the_connection_name_unhighlighted() {
        let found = filter(entries(&config()), "plock");
        assert!(found.iter().all(|entry| entry.name_match.is_empty()));
    }
}
