//! Unit tests for the shared [`GraphPath`] type.
//!
//! Split out of `path.rs` to keep that file under the 300-line guideline.

use super::path::*;
use super::types::GraphEdge;

fn edge(target: &str, rel: &str, weight: Option<f32>) -> GraphEdge {
    GraphEdge::new("ws", target, "Node", rel, weight)
}

#[test]
fn length_tracks_the_edge_sequence() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    assert_eq!(path.length(), 0);
    path.push(&edge("B", "LINK", None));
    path.push(&edge("C", "LINK", None));
    assert_eq!(path.length(), 2);
    assert_eq!(path.nodes.len(), path.edges.len() + 1);
}

#[test]
fn element_id_alternates_nodes_and_relations() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "knows", None));
    path.push(&edge("C", "follows", None));
    assert_eq!(path.element_id(), "ws:A|knows|ws:B|follows|ws:C");
}

#[test]
fn contains_node_takes_id_first_and_contains_key_takes_the_adjacency_key() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "link", None));
    assert!(path.contains_node("B", "ws"));
    assert!(!path.contains_node("ws", "B"), "order is (id, workspace)");
    assert!(path.contains_key(&("ws".to_string(), "B".to_string())));
}

#[test]
fn trail_allows_a_revisited_node_but_acyclic_does_not() {
    // A -> B -> A: two distinct edges, one repeated node.
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "out", None));
    path.push(&edge("A", "back", None));
    assert!(path.is_trail());
    assert!(!path.is_acyclic());
}

#[test]
fn a_repeated_edge_is_not_a_trail() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "link", None));
    path.push(&edge("A", "back", None));
    path.push(&edge("B", "link", None));
    assert!(!path.is_trail());
}

#[test]
fn would_repeat_edge_sees_the_pending_hop() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "link", None));
    path.push(&edge("A", "back", None));
    assert!(path.would_repeat_edge(&edge("B", "link", None)));
    assert!(!path.would_repeat_edge(&edge("B", "other", None)));
}

#[test]
fn total_weight_is_none_when_any_edge_is_unweighted() {
    let mut path = GraphPath::start(PathNode::new("A", "ws", "Node"));
    path.push(&edge("B", "link", Some(2.0)));
    assert_eq!(path.total_weight(), Some(2.0));
    path.push(&edge("C", "link", None));
    assert_eq!(path.total_weight(), None);
}

#[test]
fn with_start_node_type_only_touches_the_first_node() {
    let mut path = GraphPath::from_key(&("ws".into(), "A".into()));
    path.push(&edge("B", "link", None));
    let path = path.with_start_node_type("Account");
    assert_eq!(path.nodes[0].node_type, "Account");
    assert_eq!(path.nodes[1].node_type, "Node");
}
