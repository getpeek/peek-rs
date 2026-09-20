//! Naming a query with the local model — the port of `useLabelQuery.ts`.
//!
//! With `ai.automatically_label_queries` on, a run that finishes against an unnamed query asks
//! the model for a title and writes it to `description`, which is what the node header, page
//! search and the AI grouping prompt all read.
//!
//! Hung off the *end of a run* rather than off `isRunning` changing, which is where the
//! reference has to put it: a React effect torn down by every live-poll tick could not
//! remember it had already asked, and the ref that fixed it is the shape [`Attempts`] keeps.
//!
//! The reference also skips this for a multiplayer joiner, which has no local model of its own.
//! There is no session to ask yet; M7 has to add that guard here.

use std::collections::HashMap;

use gpui_kit::{App, Entity, Global};
use peek_canvas::Document;
use peek_document::{NodeData, NodeId, QueryData};
use peek_ollama::Chat;

use crate::node::agent::backend::Agents;
use crate::settings::Settings;

const SYSTEM_PROMPT: &str = "You name SQL queries. Given a query, reply with a short \
human-readable title describing what it returns — at most 6 words, no punctuation, \
no quotes, no SQL. Reply with the title only.";

/// The reference's `MAX_LABEL_LENGTH`.
const MAX_LABEL: usize = 60;

/// The query text each node was last asked about.
///
/// One ask per node per query text: without this a live poll against a model that answers
/// with nothing would fire a request every tick, and editing the query is what should ask for
/// a new name. It is a memo rather than document state — nothing about it belongs on disk.
#[derive(Debug, Default)]
struct Attempts(HashMap<NodeId, String>);

impl Global for Attempts {}

/// Names `node` if it has just run, has no name, and the user asked for names.
pub(crate) fn after_run(document: &Entity<Document>, node: &NodeId, cx: &mut App) {
    if !Settings::get(cx).ai.automatically_label_queries {
        return;
    }
    let Some(session) = Agents::ollama(cx) else {
        return;
    };
    let Some(data) = document
        .read(cx)
        .node(node)
        .and_then(|node| QueryData::get(&node.kind))
    else {
        return;
    };
    // A query that already says what it is does not need naming, and neither does an empty one.
    let named = data
        .description
        .as_ref()
        .is_some_and(|description| !description.trim().is_empty());
    if named || data.query.trim().is_empty() {
        return;
    }

    let query = data.query.clone();
    let asked = cx.default_global::<Attempts>().0.get(node) == Some(&query);
    if asked {
        return;
    }
    cx.default_global::<Attempts>()
        .0
        .insert(node.clone(), query.clone());

    let document = document.clone();
    let node = node.clone();
    cx.spawn(async move |cx| {
        let answer = session
            .ask(Chat::new().system(SYSTEM_PROMPT).user(query.clone()))
            .await;
        let label = match answer {
            Ok(reply) => clean(&reply),
            Err(error) => {
                log::info!("peek: the model could not name a query: {error}");
                return;
            }
        };
        if label.is_empty() {
            return;
        }
        cx.update(|cx| {
            document.update(cx, |document, cx| {
                // The query may have been edited while the model was thinking; a title for
                // text that is no longer there would be worse than none.
                let current = document
                    .node(&node)
                    .and_then(|node| QueryData::get(&node.kind))
                    .map(|data| data.query.clone());
                if current.as_deref() != Some(query.as_str()) {
                    return;
                }
                document.update_data::<QueryData>(&node, |data| data.description = Some(label));
                document.checkpoint();
                cx.notify();
            });
        });
    })
    .detach();
}

/// The first non-empty line, unquoted and cut.
///
/// A reasoning model wraps its thinking in `<think>` blocks and pads the answer with quotes or
/// a second line of commentary; the reference strips exactly this much.
fn clean(raw: &str) -> String {
    let spoken = raw.rsplit("</think>").next().unwrap_or(raw);
    spoken
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("<think>"))
        .map(|line| line.trim_matches(['"', '\'', '`']).trim())
        .map(|line| line.chars().take(MAX_LABEL).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn a_plain_answer_is_the_label() {
        assert_eq!(clean("Monthly Active Users"), "Monthly Active Users");
    }

    #[test]
    fn reasoning_quotes_and_extra_lines_are_dropped() {
        assert_eq!(
            clean(
                "<think>the user wants\na title</think>\n\n\"Churned Accounts\"\nHope that helps"
            ),
            "Churned Accounts"
        );
    }

    #[test]
    fn a_long_answer_is_cut() {
        assert_eq!(clean(&"n".repeat(200)).chars().count(), 60);
    }

    /// A model that answers with nothing must not leave the node titled with an empty string.
    #[test]
    fn an_empty_answer_is_no_label() {
        assert!(clean("   \n\n").is_empty());
        assert!(clean("<think>still thinking").is_empty());
    }
}
