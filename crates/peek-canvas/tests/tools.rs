//! The canvas tool surface, exercised the way both agents reach it: a bridge method plus a
//! camelCase params object, in and a JSON reply out.
//!
//! Error strings are asserted verbatim. They are the agent's only diagnostic, they are frozen by
//! `~/labs/peek/src/mcp/*.ts`, and a model that has learned "node X not found" should not have to
//! relearn a reworded one.

use peek_canvas::tools::{CameraMove, ToolCall, execute};
use peek_canvas::{Document, Point, Rect, Size};
use peek_document::{
    CanvasDocument, Node, NodeData, NodeId, NodeKind, NodeType, Page, QueryData, TextData,
    VariableData,
};
use serde_json::{Value, json};

fn canvas() -> Document {
    let mut persisted = CanvasDocument::empty();
    let second = Page::new("Page 2");
    persisted.page_order.push(second.id.clone());
    persisted.pages.insert(second.id.clone(), second);

    let page = persisted
        .pages
        .get_mut(&persisted.active_page_id)
        .expect("the empty document has one page");
    let mut query = Node::new(
        NodeType::Query,
        Rect::new(Point::new(0.0, 0.0), Size::new(350.0, 240.0)),
    );
    query.id = NodeId::from("query_1");
    query.kind = NodeKind::Query(QueryData::default());
    page.nodes.push(query);

    let mut text = Node::new(
        NodeType::Text,
        Rect::new(Point::new(600.0, 0.0), Size::new(100.0, 140.0)),
    );
    text.id = NodeId::from("text_1");
    text.kind = NodeKind::Text(TextData::default());
    page.nodes.push(text);

    Document::load(persisted)
}

fn call(document: &mut Document, method: &str, params: &Value) -> Value {
    execute(document, ToolCall { method, params }).reply
}

fn error(reply: &Value) -> &str {
    reply
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected an error, got {reply}"))
}

fn second_page(document: &Document) -> peek_document::PageId {
    document.neighbour_page(1).cloned().expect("two pages")
}

// ---- dispatch -----------------------------------------------------------------------------

/// The bridge renames five read tools; a local model calls them by their registry name. Both
/// spellings have to land on the same handler.
#[test]
fn a_renamed_read_tool_answers_to_either_spelling() {
    let mut document = canvas();
    let active = call(&mut document, "active_page_id", &json!({}));
    let aliased = call(&mut document, "get_active_page_id", &json!({}));
    assert_eq!(active, aliased);
    assert!(active.get("activePageId").is_some());
}

/// Never drop a reply: the bridge is parked on a one-shot channel with a five-second timeout, so
/// silence costs the agent far more than a message it can act on.
#[test]
fn an_unknown_tool_is_reported_rather_than_ignored() {
    let mut document = canvas();
    let reply = call(&mut document, "set_fire_to_the_canvas", &json!({}));
    assert_eq!(error(&reply), "unknown tool 'set_fire_to_the_canvas'");
}

/// The MCP bridge serializes an omitted argument as an explicit `null`, so every optional read
/// has to treat the two identically.
#[test]
fn an_explicit_null_reads_as_an_omitted_argument() {
    let mut document = canvas();
    let explicit = call(
        &mut document,
        "create_query_node",
        &json!({ "pageId": null, "query": "select 1", "description": null,
                 "position": null, "size": null }),
    );
    let id = NodeId::from(explicit["nodeId"].as_str().unwrap());

    let node = document.node(&id).expect("placed on the active page");
    assert_eq!(node.size(), NodeType::Query.default_size());
    assert_eq!(
        QueryData::get(&node.kind).unwrap().description,
        None,
        "a null description is absent, not Some(\"null\")"
    );
}

#[test]
fn a_required_argument_is_named_when_it_is_missing() {
    let mut document = canvas();
    let reply = call(&mut document, "create_query_node", &json!({}));
    assert_eq!(error(&reply), "missing or invalid 'query'");
}

// ---- pages --------------------------------------------------------------------------------

#[test]
fn pages_are_listed_in_display_order() {
    let mut document = canvas();
    let reply = call(&mut document, "pages", &json!({}));
    let pages = reply.as_array().expect("an array");

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["order"], json!(0));
    assert_eq!(pages[1]["order"], json!(1));
}

#[test]
fn a_page_is_created_at_the_order_asked_for_and_clamped() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "create_page",
        &json!({ "name": "wedged", "order": 0 }),
    );
    assert_eq!(reply["order"], json!(0));
    assert_eq!(reply["name"], json!("wedged"));

    let clamped = call(
        &mut document,
        "create_page",
        &json!({ "name": "last", "order": 99 }),
    );
    assert_eq!(clamped["order"], json!(3), "clamped to the page count");
}

/// Database rows must never reach a model. A bar chart embeds its rows in `data.data`, which is
/// the only place a page can carry them — result rows live in the sidecar, outside the page.
#[test]
fn page_content_strips_embedded_rows() {
    let mut document = canvas();
    let chart = document.create_node(
        NodeType::Barchart,
        Rect::new(Point::new(0.0, 0.0), Size::new(300.0, 200.0)),
    );
    let page = document.active_page_id().clone();

    let reply = call(
        &mut document,
        "page_content",
        &json!({ "pageId": page.as_str() }),
    );
    let nodes = reply["nodes"].as_array().expect("nodes");
    let rendered = nodes
        .iter()
        .find(|node| node["id"] == json!(chart.as_str()))
        .expect("the chart is there");

    assert!(rendered["data"].get("data").is_none(), "rows were stripped");
    assert!(
        reply.get("viewport").is_some(),
        "the rest of the page survives"
    );
}

#[test]
fn an_unknown_page_is_named_in_the_error() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "page_content",
        &json!({ "pageId": "page_nope" }),
    );
    assert_eq!(error(&reply), "page page_nope not found");
}

// ---- nodes --------------------------------------------------------------------------------

#[test]
fn creating_a_query_node_reveals_the_page_it_lands_on() {
    let mut document = canvas();
    let second = second_page(&document);

    let reply = call(
        &mut document,
        "create_query_node",
        &json!({ "pageId": second.as_str(), "query": "select 2",
                 "position": [10.0, 20.0], "size": [400.0, 300.0] }),
    );

    assert_eq!(reply["pageId"], json!(second.as_str()));
    assert_eq!(
        document.active_page_id(),
        &second,
        "a create reveals its page"
    );
    let id = NodeId::from(reply["nodeId"].as_str().unwrap());
    let node = document.node(&id).expect("placed");
    assert_eq!(node.position, Point::new(10.0, 20.0));
    assert_eq!(QueryData::get(&node.kind).unwrap().query, "select 2");
}

/// The opposite rule: tidying a node the user is not looking at must not yank the view to it.
#[test]
fn updating_a_node_on_another_page_leaves_the_view_alone() {
    let mut document = canvas();
    let second = second_page(&document);
    let created = call(
        &mut document,
        "create_query_node",
        &json!({ "pageId": second.as_str(), "query": "select 2" }),
    );
    let id = created["nodeId"].as_str().unwrap().to_string();

    let first = document.active_page_id().clone();
    document.switch_page(&first);

    let reply = call(
        &mut document,
        "update_query_node",
        &json!({ "nodeId": id, "query": "select 3" }),
    );

    assert_eq!(reply["pageId"], json!(second.as_str()));
    assert_eq!(document.active_page_id(), &first, "the view stayed put");

    let edited = document
        .on_page(&second, |document| {
            QueryData::get(&document.node(&NodeId::from(id.as_str())).unwrap().kind)
                .unwrap()
                .query
                .clone()
        })
        .unwrap();
    assert_eq!(edited, "select 3");
}

#[test]
fn naming_the_wrong_kind_of_node_fails_without_writing() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "update_query_node",
        &json!({ "nodeId": "text_1", "query": "select 1" }),
    );
    assert_eq!(error(&reply), "node text_1 is not a query node");
    assert_eq!(
        TextData::get(&document.node(&NodeId::from("text_1")).unwrap().kind)
            .unwrap()
            .text,
        "",
        "nothing was half-applied"
    );
}

#[test]
fn an_unknown_node_is_named_in_the_error() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "update_text_node",
        &json!({ "nodeId": "text_nope", "text": "hi" }),
    );
    assert_eq!(error(&reply), "node text_nope not found");
}

#[test]
fn a_text_node_keeps_its_width_when_its_height_changes() {
    let mut document = canvas();
    call(
        &mut document,
        "update_text_node",
        &json!({ "nodeId": "text_1", "height": 48.0 }),
    );
    let node = document.node(&NodeId::from("text_1")).unwrap();
    // Compared whole: the width follows the text, not the tool, so only the height moved.
    assert_eq!(node.size(), Size::new(100.0, 48.0));
}

#[test]
fn variables_are_validated_before_a_node_is_created() {
    let mut document = canvas();
    let before = document.nodes().len();

    let empty = call(
        &mut document,
        "create_vars_node",
        &json!({ "variables": {} }),
    );
    assert_eq!(error(&empty), "variables map is empty");

    let bad = call(
        &mut document,
        "create_vars_node",
        &json!({ "variables": { "2limit": "5" } }),
    );
    assert_eq!(error(&bad), "invalid variable name: 2limit");
    assert_eq!(document.nodes().len(), before, "nothing was created");
}

/// `serde_json` is built with `preserve_order`, so the rows land the way the agent wrote them
/// rather than alphabetically — the order is visible in the node and worth keeping.
#[test]
fn variable_rows_keep_the_order_they_arrived_in() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "create_vars_node",
        &json!({ "variables": { "zebra": "1", "apple": ["2", "3"] } }),
    );
    let id = NodeId::from(reply["nodeId"].as_str().unwrap());
    let data = VariableData::get(&document.node(&id).unwrap().kind).unwrap();

    let names: Vec<&str> = data.rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, vec!["zebra", "apple"]);
}

#[test]
fn a_global_variable_node_wires_itself_to_every_query_on_its_page() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "create_vars_node",
        &json!({ "variables": { "limit": "5" }, "global": true }),
    );
    let id = NodeId::from(reply["nodeId"].as_str().unwrap());

    assert!(
        document
            .edges()
            .iter()
            .any(|edge| edge.source == id && edge.target == NodeId::from("query_1"))
    );
}

/// One call is one action, so the node and every edge it drew undo together.
#[test]
fn a_global_variable_node_and_its_edges_are_one_undo_step() {
    let mut document = canvas();
    let nodes = document.nodes().len();

    call(
        &mut document,
        "create_vars_node",
        &json!({ "variables": { "limit": "5" }, "global": true }),
    );
    assert_eq!(document.edges().len(), 1);

    assert!(document.undo());
    assert_eq!(document.nodes().len(), nodes);
    assert!(document.edges().is_empty(), "the edge went with the node");
}

#[test]
fn connecting_reports_the_edge_and_is_idempotent() {
    let mut document = canvas();
    let params = json!({ "from": "query_1", "to": "text_1" });

    let first = call(&mut document, "connect_nodes", &params);
    assert_eq!(first["edgeId"], json!("query_1->text_1"));

    let again = call(&mut document, "connect_nodes", &params);
    assert_eq!(
        again, first,
        "an existing edge is a success, not a conflict"
    );
    assert_eq!(document.edges().len(), 1);
}

#[test]
fn connecting_refuses_a_self_edge_and_a_cross_page_edge() {
    let mut document = canvas();
    let itself = call(
        &mut document,
        "connect_nodes",
        &json!({ "from": "query_1", "to": "query_1" }),
    );
    assert_eq!(error(&itself), "cannot connect a node to itself");

    let second = second_page(&document);
    let created = call(
        &mut document,
        "create_query_node",
        &json!({ "pageId": second.as_str(), "query": "select 2" }),
    );
    let far = created["nodeId"].as_str().unwrap();

    let reply = call(
        &mut document,
        "connect_nodes",
        &json!({ "from": "query_1", "to": far }),
    );
    assert!(
        error(&reply).starts_with("nodes are on different pages ("),
        "got {}",
        error(&reply)
    );
}

// ---- regions ------------------------------------------------------------------------------

#[test]
fn grouping_needs_a_name_and_at_least_two_nodes() {
    let mut document = canvas();

    let blank = call(
        &mut document,
        "group_nodes",
        &json!({ "name": "   ", "nodeIds": ["query_1", "text_1"] }),
    );
    assert_eq!(error(&blank), "name is required");

    let lonely = call(
        &mut document,
        "group_nodes",
        &json!({ "name": "solo", "nodeIds": ["query_1"] }),
    );
    assert_eq!(error(&lonely), "pass at least two node ids to group");
}

#[test]
fn grouping_names_the_nodes_that_are_not_on_the_page() {
    let mut document = canvas();
    let reply = call(
        &mut document,
        "group_nodes",
        &json!({ "name": "wide", "nodeIds": ["query_1", "nope_1"] }),
    );
    let page = document.active_page_id();
    assert_eq!(error(&reply), format!("nodes not on page {page}: nope_1"));
}

#[test]
fn a_group_is_a_suggestion_unless_the_caller_says_otherwise() {
    let mut document = canvas();

    call(
        &mut document,
        "group_nodes",
        &json!({ "name": "reviewed", "nodeIds": ["query_1", "text_1"], "suggested": false }),
    );
    assert_eq!(
        call(&mut document, "list_regions", &json!({}))["regions"][0]["status"],
        json!("confirmed")
    );

    let mut fresh = canvas();
    call(
        &mut fresh,
        "group_nodes",
        &json!({ "name": "proposed", "nodeIds": ["query_1", "text_1"] }),
    );
    assert_eq!(
        call(&mut fresh, "list_regions", &json!({}))["regions"][0]["status"],
        json!("suggested")
    );
}

/// A drawing is annotation, not work waiting to be organised, so it never shows up as ungrouped.
#[test]
fn listing_regions_reports_ungrouped_nodes_but_not_drawings() {
    let mut document = canvas();
    document.create_node(
        NodeType::Draw,
        Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0)),
    );
    call(
        &mut document,
        "group_nodes",
        &json!({ "name": "pair", "nodeIds": ["query_1", "text_1"] }),
    );

    let reply = call(&mut document, "list_regions", &json!({}));
    assert_eq!(reply["regions"][0]["nodeIds"], json!(["query_1", "text_1"]));
    assert_eq!(
        reply["ungroupedNodeIds"],
        json!([]),
        "the drawing is not work"
    );
}

#[test]
fn a_region_is_found_by_id_without_naming_its_page() {
    let mut document = canvas();
    let grouped = call(
        &mut document,
        "group_nodes",
        &json!({ "name": "pair", "nodeIds": ["query_1", "text_1"] }),
    );
    let region = grouped["regionId"].as_str().unwrap();

    let removed = call(
        &mut document,
        "remove_region",
        &json!({ "regionId": region }),
    );
    assert_eq!(removed["regionId"], json!(region));
    assert!(document.regions().is_empty());

    let gone = call(
        &mut document,
        "remove_region",
        &json!({ "regionId": region }),
    );
    assert_eq!(error(&gone), format!("region {region} not found"));
}

#[test]
fn adding_to_a_region_needs_at_least_one_node() {
    let mut document = canvas();
    let grouped = call(
        &mut document,
        "group_nodes",
        &json!({ "name": "pair", "nodeIds": ["query_1", "text_1"] }),
    );
    let reply = call(
        &mut document,
        "add_to_region",
        &json!({ "regionId": grouped["regionId"], "nodeIds": [] }),
    );
    assert_eq!(error(&reply), "pass at least one node id to add");
}

// ---- camera and selection -----------------------------------------------------------------

#[test]
fn the_camera_tools_hand_the_view_a_target() {
    let mut document = canvas();

    let panned = execute(
        &mut document,
        ToolCall {
            method: "camera_pan_to",
            params: &json!({ "position": [10.0, 20.0] }),
        },
    );
    assert_eq!(
        panned.camera,
        Some(CameraMove::PanTo(Point::new(10.0, 20.0)))
    );

    let fitted = execute(
        &mut document,
        ToolCall {
            method: "camera_fit_node",
            params: &json!({ "nodeId": "query_1" }),
        },
    );
    assert_eq!(
        fitted.camera,
        Some(CameraMove::Fit(
            document.node(&NodeId::from("query_1")).unwrap().bounds()
        ))
    );
}

#[test]
fn the_zoom_is_clamped_to_the_canvas_range_and_reported_back() {
    let mut document = canvas();
    let outcome = execute(
        &mut document,
        ToolCall {
            method: "camera_set_zoom",
            params: &json!({ "zoom": 99.0 }),
        },
    );

    assert_eq!(
        outcome.camera,
        Some(CameraMove::Zoom(peek_canvas::MAX_ZOOM))
    );
    assert_eq!(outcome.reply["zoom"], json!(peek_canvas::MAX_ZOOM));
}

#[test]
fn selecting_switches_to_the_nodes_page_and_drops_ids_that_are_not_on_it() {
    let mut document = canvas();
    let second = second_page(&document);
    let created = call(
        &mut document,
        "create_query_node",
        &json!({ "pageId": second.as_str(), "query": "select 2" }),
    );
    let far = created["nodeId"].as_str().unwrap().to_string();
    let first = document.active_page_id().clone();
    document.switch_page(&first);

    let reply = call(
        &mut document,
        "select_nodes",
        &json!({ "nodeIds": [far.as_str(), "query_1"] }),
    );

    assert_eq!(document.active_page_id(), &second);
    assert_eq!(reply["selected"], json!([far.as_str()]));
    assert_eq!(reply["pageId"], json!(second.as_str()));
}

#[test]
fn selecting_nothing_clears_the_selection() {
    let mut document = canvas();
    document.select_only([NodeId::from("query_1")]);

    let reply = call(&mut document, "select_nodes", &json!({ "nodeIds": [] }));
    assert!(document.selected().is_empty());
    assert_eq!(reply["selected"], json!([]));
}

// ---- the agent adapter ---------------------------------------------------------------------

/// The agent schema omits what a small model gets wrong; the adapter fills it in so the executor
/// sees the same complete arguments the MCP bridge sends.
mod agent_adapter {
    use super::{Document, Node, NodeId, NodeKind, NodeType, Point, Rect, Size, json};
    use peek_canvas::tools::{ToolCall, agent_params};
    use peek_document::{AgentData, CanvasDocument};
    use serde_json::Value;

    fn canvas() -> Document {
        let mut persisted = CanvasDocument::empty();
        let page = persisted
            .pages
            .get_mut(&persisted.active_page_id)
            .expect("one page");
        let mut agent = Node::new(
            NodeType::Agent,
            Rect::new(Point::new(100.0, 200.0), Size::new(540.0, 400.0)),
        );
        agent.id = NodeId::from("agent_1");
        agent.kind = NodeKind::Agent(AgentData::default());
        page.nodes.push(agent);
        Document::load(persisted)
    }

    fn filled(document: &Document, method: &str, params: &Value) -> Value {
        agent_params(
            document,
            &NodeId::from("agent_1"),
            ToolCall { method, params },
        )
    }

    #[test]
    fn a_placed_node_lands_to_the_right_of_the_agent_that_asked_for_it() {
        let document = canvas();
        let params = filled(
            &document,
            "create_query_node",
            &json!({ "query": "select 1" }),
        );

        assert_eq!(params["position"], json!([100.0 + 540.0 + 80.0, 200.0]));
        assert_eq!(params["size"], json!([350.0, 240.0]));
    }

    /// The row comes from the agent's outgoing-edge count, so a second query stacks below the
    /// first rather than on top of it.
    #[test]
    fn successive_nodes_step_down_a_row_each() {
        let mut document = canvas();
        let query = document.create_node(
            NodeType::Query,
            Rect::new(Point::new(0.0, 0.0), Size::new(350.0, 240.0)),
        );
        document.connect(&NodeId::from("agent_1"), &query);

        let params = filled(
            &document,
            "create_query_node",
            &json!({ "query": "select 1" }),
        );
        assert_eq!(params["position"], json!([720.0, 200.0 + 240.0 + 40.0]));
    }

    #[test]
    fn what_the_model_did_supply_wins() {
        let document = canvas();
        let params = filled(
            &document,
            "create_query_node",
            &json!({ "query": "select 1", "position": [1.0, 2.0], "size": [10.0, 20.0] }),
        );

        assert_eq!(params["position"], json!([1.0, 2.0]));
        assert_eq!(params["size"], json!([10.0, 20.0]));
    }

    /// A text node's height is its font size, and it has no `size` at all.
    #[test]
    fn a_text_node_gets_a_height_rather_than_a_size() {
        let document = canvas();
        let params = filled(&document, "create_text_node", &json!({ "text": "hello" }));

        assert_eq!(
            params["height"],
            json!(NodeType::Text.default_size().height)
        );
        assert!(params.get("size").is_none());
    }

    #[test]
    fn a_page_defaults_to_the_front_of_the_list() {
        let document = canvas();
        let params = filled(&document, "create_page", &json!({ "name": "new" }));
        assert_eq!(params["order"], json!(0));
    }

    /// Tools that place nothing are passed through untouched.
    #[test]
    fn a_read_tool_is_left_alone() {
        let document = canvas();
        let params = filled(&document, "get_pages", &json!({}));
        assert_eq!(params, json!({}));
    }
}
