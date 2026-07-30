//! Unit tests for [`super::traversal`].
//!
//! Split out to keep the implementation file under the 300-line limit; it is
//! `#[path]`-included so the tests keep access to private items.

use super::*;

fn edge(target: &str, node_type: &str, rel: &str) -> GraphEdge {
    GraphEdge::new("ws", target, node_type, rel, None)
}

fn params(min: u32, max: u32, restrictor: PathRestrictor) -> TraversalParams {
    TraversalParams {
        min_depth: min,
        max_depth: max,
        restrictor,
        target_labels: vec![],
    }
}

/// a -> b -> c, plus b -> a so the graph has a cycle.
fn cyclic_graph() -> GraphAdjacency {
    let mut adj: GraphAdjacency = HashMap::new();
    adj.insert(("ws".into(), "a".into()), vec![edge("b", "T", "knows")]);
    adj.insert(
        ("ws".into(), "b".into()),
        vec![edge("c", "T", "knows"), edge("a", "T", "knows")],
    );
    adj
}

fn start() -> GraphNodeId {
    ("ws".into(), "a".into())
}

#[test]
fn acyclic_is_the_default_and_never_revisits_a_node() {
    let mut budget = MAX_PATHS;
    let paths = enumerate_paths(
        &start(),
        "T",
        &cyclic_graph(),
        &params(1, 5, PathRestrictor::Acyclic),
        &mut budget,
    )
    .unwrap();
    assert!(paths.iter().all(|p| p.is_acyclic()));
    // a->b and a->b->c only; a->b->a is rejected.
    assert_eq!(paths.len(), 2);
}

#[test]
fn walk_permits_revisiting_and_is_bounded_only_by_depth() {
    let mut budget = MAX_PATHS;
    let paths = enumerate_paths(
        &start(),
        "T",
        &cyclic_graph(),
        &params(1, 4, PathRestrictor::Walk),
        &mut budget,
    )
    .unwrap();
    assert!(paths.iter().any(|p| !p.is_acyclic()));
    assert!(paths.iter().all(|p| p.length() <= 4));
}

#[test]
fn trail_permits_revisiting_a_node_via_a_different_edge() {
    let mut adj: GraphAdjacency = HashMap::new();
    adj.insert(("ws".into(), "a".into()), vec![edge("b", "T", "one")]);
    adj.insert(("ws".into(), "b".into()), vec![edge("a", "T", "two")]);

    let mut budget = MAX_PATHS;
    let paths = enumerate_paths(
        &start(),
        "T",
        &adj,
        &params(1, 3, PathRestrictor::Trail),
        &mut budget,
    )
    .unwrap();
    assert!(paths.iter().all(|p| p.is_trail()));
    assert!(paths.iter().any(|p| p.length() == 2 && !p.is_acyclic()));
}

#[test]
fn exceeding_the_cap_errors_instead_of_truncating() {
    let mut budget = 1;
    let err = enumerate_paths(
        &start(),
        "T",
        &cyclic_graph(),
        &params(1, 5, PathRestrictor::Acyclic),
        &mut budget,
    )
    .unwrap_err();
    assert!(err.to_string().contains("path cap"), "{err}");
}

#[test]
fn target_labels_filter_the_final_node_only() {
    let mut budget = MAX_PATHS;
    let mut p = params(1, 5, PathRestrictor::Acyclic);
    p.target_labels = vec!["Other".into()];
    let paths = enumerate_paths(&start(), "T", &cyclic_graph(), &p, &mut budget).unwrap();
    assert!(paths.is_empty());
}

/// The regression this pass fixes: `-[:knows|follows]->` used to push only
/// `types.first()` to storage, so every `follows` edge vanished.
#[test]
fn multi_type_alternation_keeps_every_named_type() {
    let relations = vec![
        (
            "ws".to_string(),
            "a".to_string(),
            "ws".to_string(),
            "b".to_string(),
            relation("knows"),
        ),
        (
            "ws".to_string(),
            "a".to_string(),
            "ws".to_string(),
            "c".to_string(),
            relation("follows"),
        ),
        (
            "ws".to_string(),
            "a".to_string(),
            "ws".to_string(),
            "d".to_string(),
            relation("blocks"),
        ),
    ];

    let (forward, _) = build_adjacency(&relations, &["knows".into(), "follows".into()]);
    let out = &forward[&("ws".to_string(), "a".to_string())];
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|e| e.relation_type == "knows"));
    assert!(out.iter().any(|e| e.relation_type == "follows"));
    assert!(!out.iter().any(|e| e.relation_type == "blocks"));
}

#[test]
fn an_empty_type_list_accepts_every_relation() {
    let relations = vec![(
        "ws".to_string(),
        "a".to_string(),
        "ws".to_string(),
        "b".to_string(),
        relation("anything"),
    )];
    let (forward, reverse) = build_adjacency(&relations, &[]);
    assert_eq!(forward.len(), 1);
    assert_eq!(reverse.len(), 1);
}

fn relation(relation_type: &str) -> FullRelation {
    FullRelation {
        source_id: "a".into(),
        source_workspace: "ws".into(),
        source_node_type: "T".into(),
        target_id: "b".into(),
        target_workspace: "ws".into(),
        target_node_type: "T".into(),
        relation_type: relation_type.into(),
        weight: None,
    }
}
