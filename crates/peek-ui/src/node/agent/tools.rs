//! What a local model is told it can do.
//!
//! The same twenty-one operations `peek-mcp` advertises, but described the way
//! `~/labs/peek/src/canvas/nodes/Agent/agentTools.ts` describes them: terser, no `pageId`, and
//! with geometry optional. A small model places nodes badly and has no business choosing a
//! page, so [`peek_canvas::tools::agent_params`] fills those in before the call is executed.
//!
//! Only the schemas live here. The executor is `peek_canvas::tools`, shared with the MCP bridge,
//! because two implementations of "create a query node" would be two things to keep in step.

use peek_db::Engine;
use peek_ollama::Tool;
use serde_json::{Value, json};

/// The system prompt, ported from `agentTools.ts`.
///
/// The first paragraph is load-bearing: the agent cannot run queries, and a model that believes
/// it can will happily invent rows it never saw.
pub(super) fn system_prompt(engine: Engine) -> String {
    format!(
        "You are Peek's canvas agent. You help the user explore a SQL database and build on an \
         infinite canvas of nodes.\n\n\
         You cannot execute queries yourself. When data is needed, use create_query_node to \
         place an un-run query node on the canvas; the user runs it themselves and links the \
         Result node back. Never claim to have run a query or to have seen its rows.\n\n\
         Other tools build and arrange the board (create_vars_node, create_text_node, \
         create_page, update_query_node, update_vars_node, update_text_node, connect_nodes), \
         organize it into named regions (group_nodes, list_regions, add_to_region, \
         remove_region) and drive the view (camera_pan_to, camera_set_zoom, camera_fit_node, \
         select_nodes). Read tools (get_db_schema, get_connection_info, get_active_page_id, \
         get_pages, get_page_content) inspect the current state.\n\n\
         Guidance:\n\
         - Call get_db_schema when you need table or column names rather than guessing; it \
         isn't given to you up front.\n\
         - When you create a node you may omit position/size; it is placed next to you \
         automatically.\n\
         - Prefer a direct answer or analysis over a tool call. Only use a tool when it is \
         necessary.\n\
         - After a tool returns, use the result to answer the user. Never repeat the same tool \
         call with the same arguments.\n\
         - Regions are a living document. Before changing groups call list_regions, then \
         reorganize with the least disruptive tool: add_to_region to fold loose nodes into a \
         fitting group, group_nodes to start or reshape one, remove_region to drop one.\n\
         - Write valid {}.",
        engine.dialect_name()
    )
}

fn position() -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "description": "[x, y] in flow coords; omit to auto-place next to the agent"
    })
}

fn size() -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "description": "[width, height] in flow coords; omit for a sensible default"
    })
}

fn variables() -> Value {
    json!({
        "type": "object",
        "description": "map of variable name → value (string or list of strings)"
    })
}

fn node_ids(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

fn text(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn schema(properties: Value, required: &[&str]) -> Value {
    Value::Object(serde_json::Map::from_iter([
        ("type".to_string(), Value::from("object")),
        ("properties".to_string(), properties),
        ("required".to_string(), json!(required)),
    ]))
}

/// Every tool the agent node offers, in the reference's order.
pub(super) fn all() -> Vec<Tool> {
    let mut tools = creates();
    tools.extend(edits());
    tools.extend(regions());
    tools.extend(view());
    tools.extend(reads());
    tools
}

/// Putting new things on the canvas.
fn creates() -> Vec<Tool> {
    vec![
        Tool::function(
            "create_query_node",
            "Place an un-run SQL query node on the canvas for the user to run themselves. Use \
             when the user asks you to create/write/add a query. The agent cannot execute \
             queries — the user runs them and links the result back.",
            schema(
                json!({
                    "query": text("a valid SQL query in the connected database's dialect"),
                    "description": text("short human-readable title shown on the node"),
                    "position": position(),
                    "size": size(),
                }),
                &["query"],
            ),
        ),
        Tool::function(
            "create_vars_node",
            "Create a Variable node holding reusable named values that queries reference with \
             @name. Set global to wire it to every query on the page.",
            schema(
                json!({
                    "variables": variables(),
                    "global": { "type": "boolean" },
                    "position": position(),
                    "size": size(),
                }),
                &["variables"],
            ),
        ),
        Tool::function(
            "create_text_node",
            "Create a free-form text caption. Its height sets the font size (tall = heading, \
             short = small label); width auto-fits the single-line text.",
            schema(
                json!({
                    "text": text("the caption"),
                    "height": { "type": "number", "description": "font size; taller is bigger" },
                    "position": position(),
                }),
                &["text"],
            ),
        ),
        Tool::function(
            "create_page",
            "Create a new empty page and switch to it.",
            schema(
                json!({
                    "name": text("name for the new page"),
                    "order": { "type": "number", "description": "0-based insert position" },
                }),
                &["name"],
            ),
        ),
    ]
}

/// Changing and wiring what is already there.
fn edits() -> Vec<Tool> {
    vec![
        Tool::function(
            "update_query_node",
            "Edit an existing query node found by nodeId. Only the fields you pass change.",
            schema(
                json!({
                    "nodeId": text("id of the query node"),
                    "query": text("the replacement SQL"),
                    "description": text("short human-readable title shown on the node"),
                    "position": position(),
                    "size": size(),
                }),
                &["nodeId"],
            ),
        ),
        Tool::function(
            "update_vars_node",
            "Edit an existing variable node found by nodeId. Passing variables replaces the \
             whole map; global: true auto-wires it to every query on its page.",
            schema(
                json!({
                    "nodeId": text("id of the variable node"),
                    "variables": variables(),
                    "global": { "type": "boolean" },
                    "position": position(),
                    "size": size(),
                }),
                &["nodeId"],
            ),
        ),
        Tool::function(
            "update_text_node",
            "Edit an existing text node found by nodeId. Only the fields you pass change.",
            schema(
                json!({
                    "nodeId": text("id of the text node"),
                    "text": text("the replacement caption"),
                    "height": { "type": "number", "description": "font size" },
                    "position": position(),
                }),
                &["nodeId"],
            ),
        ),
        Tool::function(
            "connect_nodes",
            "Draw an edge from one node to another (both must be on the same page). E.g. attach \
             a variable node to a query.",
            schema(
                json!({ "from": text("source node id"), "to": text("target node id") }),
                &["from", "to"],
            ),
        ),
    ]
}

/// Wayfinding: the labels a reader sees when they zoom out.
fn regions() -> Vec<Tool> {
    vec![
        Tool::function(
            "group_nodes",
            "Group nodes into a named region — a wayfinding label the user sees when zoomed \
             out. Group by MEANING, not edge connectivity: connected subgraphs often cover \
             different questions as an exploration drills down, so prefer several precise \
             regions over one broad one. A node belongs to one region; grouping claims it from \
             any previous region. Created as a suggestion the user reviews unless \
             suggested=false.",
            schema(
                json!({
                    "nodeIds": node_ids("ids of the nodes to group (at least two)"),
                    "name": text("short region name, 2-4 words"),
                    "desc": text("one-line description of the group"),
                    "suggested": {
                        "type": "boolean",
                        "description": "false to skip the user's review step"
                    },
                }),
                &["nodeIds", "name"],
            ),
        ),
        Tool::function(
            "list_regions",
            "List the active page's regions and the node ids not in any region. The canvas is a \
             living document — call this first, then reorganize: grow a region with \
             add_to_region, start or reshape one with group_nodes, or drop one with \
             remove_region.",
            schema(json!({}), &[]),
        ),
        Tool::function(
            "add_to_region",
            "Add nodes to an EXISTING region (found by regionId) without creating a new one — \
             use this to grow a region as the canvas evolves. A node belongs to one region, so \
             the nodes are claimed from any region that already holds them. The region's name \
             and description are kept.",
            schema(
                json!({
                    "regionId": text("id of the region to add to, from list_regions"),
                    "nodeIds": node_ids("ids of the nodes to add (at least one)"),
                }),
                &["regionId", "nodeIds"],
            ),
        ),
        Tool::function(
            "remove_region",
            "Delete a region by regionId. Member nodes are kept — they become ungrouped.",
            schema(
                json!({ "regionId": text("id of the region") }),
                &["regionId"],
            ),
        ),
    ]
}

/// Moving the camera and the selection.
fn view() -> Vec<Tool> {
    vec![
        Tool::function(
            "camera_pan_to",
            "Center the camera on a point [x, y] in flow coords, keeping the current zoom.",
            schema(
                json!({ "position": { "type": "array", "items": { "type": "number" } } }),
                &["position"],
            ),
        ),
        Tool::function(
            "camera_set_zoom",
            "Set the camera zoom (1.0 = 100%), clamped to 0.1–4.0.",
            schema(json!({ "zoom": { "type": "number" } }), &["zoom"]),
        ),
        Tool::function(
            "camera_fit_node",
            "Frame a node by nodeId, switching to its page if needed.",
            schema(
                json!({ "nodeId": text("id of the node to frame") }),
                &["nodeId"],
            ),
        ),
        Tool::function(
            "select_nodes",
            "Replace the current selection with the given node ids (empty clears it).",
            schema(
                json!({ "nodeIds": node_ids("ids of the nodes to select") }),
                &["nodeIds"],
            ),
        ),
    ]
}

/// Reads. Nothing here mutates the document, and none of it returns database rows.
fn reads() -> Vec<Tool> {
    vec![
        Tool::function(
            "get_db_schema",
            "Get the active connection's schema as compact DDL, one line per table: \
             `table(col type PK, fk_col type ->ref_table.col, ...)`. Pass tables to fetch only \
             those; omit for the whole schema.",
            schema(
                json!({ "tables": node_ids("return only these tables; omit for the full schema") }),
                &[],
            ),
        ),
        Tool::function(
            "get_connection_info",
            "Get the active connection's { name, engine }. Never returns the URL or credentials.",
            schema(json!({}), &[]),
        ),
        Tool::function(
            "get_active_page_id",
            "Get the id of the currently active page.",
            schema(json!({}), &[]),
        ),
        Tool::function(
            "get_pages",
            "List every page on the current connection as [{ id, name, order }].",
            schema(json!({}), &[]),
        ),
        Tool::function(
            "get_page_content",
            "Get a page's nodes/edges/viewport by pageId (embedded data rows are stripped).",
            schema(json!({ "pageId": text("id of the page") }), &["pageId"]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{all, system_prompt};
    use peek_db::Engine;

    /// The same twenty-one the MCP bridge advertises: one surface, two descriptions of it.
    #[test]
    fn every_tool_is_offered_exactly_once() {
        let tools = all();
        assert_eq!(tools.len(), 21);

        let mut names: Vec<&str> = tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "a tool is offered twice");
    }

    /// Every parameter the model may pass has to be described, or it guesses at the shape.
    #[test]
    fn every_tool_describes_its_arguments() {
        for tool in all() {
            let parameters = &tool.function.parameters;
            assert_eq!(parameters["type"], "object", "{}", tool.function.name);
            assert!(
                parameters.get("properties").is_some(),
                "{} has no properties",
                tool.function.name
            );
            assert!(
                !tool.function.description.is_empty(),
                "{} has no description",
                tool.function.name
            );
        }
    }

    #[test]
    fn the_prompt_names_the_dialect_and_the_one_thing_the_agent_cannot_do() {
        let prompt = system_prompt(Engine::Postgres);
        assert!(prompt.contains("PostgreSQL"));
        assert!(prompt.contains("You cannot execute queries yourself"));
    }
}
