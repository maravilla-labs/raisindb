//! Unit tests for [`super::variable_length`].
//!
//! Split out to keep the implementation file under the 300-line limit; it is
//! `#[path]`-included so the tests keep access to private items.

use super::*;
use crate::physical_plan::graph_algo::{GraphEdge, PathNode};
use raisin_sql::ast::SourceSpan;

fn rel(variable: Option<&str>) -> RelationshipPattern {
    RelationshipPattern {
        variable: variable.map(|s| s.to_string()),
        types: vec!["knows".into()],
        direction: Direction::Right,
        quantifier: Some(PathQuantifier::range(1, 3)),
        cost: None,
        span: SourceSpan::empty(),
    }
}

fn node(variable: Option<&str>) -> NodePattern {
    NodePattern {
        variable: variable.map(|s| s.to_string()),
        labels: vec![],
        span: SourceSpan::empty(),
    }
}

fn two_hop() -> GraphPath {
    let mut path = GraphPath::start(PathNode::new("a", "ws", "User"));
    path.push(&GraphEdge::new("ws", "b", "User", "knows", None));
    path.push(&GraphEdge::new("ws", "c", "User", "knows", None));
    path
}

#[test]
fn relation_type_is_bound_verbatim_not_length_encoded() {
    let mut semantics = PathSemantics::legacy();
    semantics.variable = Some("p".into());

    let binding = bind_path(
        &VariableBinding::new(),
        &two_hop(),
        "a",
        &rel(Some("r")),
        &node(Some("b")),
        &semantics,
    );

    let rel_info = binding.get_relation("r").expect("relation bound");
    assert_eq!(rel_info.relation_type, "knows");
    assert!(!rel_info.relation_type.contains('['));
}

#[test]
fn the_path_is_bound_under_both_the_path_and_relation_variables() {
    let mut semantics = PathSemantics::legacy();
    semantics.variable = Some("p".into());

    let binding = bind_path(
        &VariableBinding::new(),
        &two_hop(),
        "a",
        &rel(Some("r")),
        &node(Some("b")),
        &semantics,
    );

    assert_eq!(binding.get_path("p").map(GraphPath::length), Some(2));
    assert_eq!(binding.get_path("r").map(GraphPath::length), Some(2));
    assert_eq!(
        binding.get_node("a").map(|n| n.id.clone()),
        Some("a".into())
    );
    assert_eq!(
        binding.get_node("b").map(|n| n.id.clone()),
        Some("c".into())
    );
}
