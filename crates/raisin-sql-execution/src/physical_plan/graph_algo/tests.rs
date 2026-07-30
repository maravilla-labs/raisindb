//! Unit tests for the shared graph path algorithms.
//!
//! The load-bearing case is [`cheapest_route_is_not_the_fewest_hops`]: a
//! hop-count implementation passes a weighted test *accidentally* whenever the
//! cheapest route happens to also be the shortest one, so every weighted test
//! here is built so that the two answers differ.

use super::cost::{CostError, CostSpec};
use super::{
    all_shortest_paths, cheapest_path, k_shortest_paths, shortest_path, GraphAdjacency, GraphEdge,
    GraphNodeId,
};
use std::collections::HashMap;

fn node(id: &str) -> GraphNodeId {
    ("ws".to_string(), id.to_string())
}

/// Build an adjacency list from `(source, target, relation_type, weight)` rows.
fn graph(rows: &[(&str, &str, &str, Option<f32>)]) -> GraphAdjacency {
    let mut adjacency: GraphAdjacency = HashMap::new();
    for (src, tgt, rel, weight) in rows {
        adjacency
            .entry(node(src))
            .or_default()
            .push(GraphEdge::new("ws", *tgt, "Node", *rel, *weight));
        adjacency.entry(node(tgt)).or_default();
    }
    adjacency
}

fn ids(path: &super::GraphPath) -> Vec<String> {
    path.nodes.iter().map(|n| n.id.clone()).collect()
}

// ---------------------------------------------------------------------------
// Hop-count shortest path
// ---------------------------------------------------------------------------

/// Linear A->B->C->D plus a shortcut A->C, so the minimum is 2 hops.
fn linear_with_shortcut() -> GraphAdjacency {
    graph(&[
        ("A", "B", "LINK", None),
        ("B", "C", "LINK", None),
        ("C", "D", "LINK", None),
        ("A", "C", "SHORT", None),
    ])
}

#[test]
fn shortest_path_returns_the_minimum_hop_route() {
    let path =
        shortest_path(&linear_with_shortcut(), &node("A"), &node("D"), 10).expect("A reaches D");
    assert_eq!(path.length(), 2);
    assert_eq!(ids(&path), vec!["A", "C", "D"]);
}

#[test]
fn shortest_path_carries_the_full_ordered_sequence() {
    let path = shortest_path(&linear_with_shortcut(), &node("A"), &node("D"), 10).unwrap();
    assert_eq!(path.nodes.len(), path.edges.len() + 1);
    assert_eq!(
        path.edges
            .iter()
            .map(|e| e.relation_type.as_str())
            .collect::<Vec<_>>(),
        vec!["SHORT", "LINK"]
    );
    assert_eq!(path.edges[0].source_id, "A");
    assert_eq!(path.edges[0].target_id, "C");
}

#[test]
fn shortest_path_returns_none_when_unreachable() {
    assert!(shortest_path(&linear_with_shortcut(), &node("D"), &node("A"), 10).is_none());
}

#[test]
fn shortest_path_to_self_is_a_zero_hop_path() {
    let path = shortest_path(&linear_with_shortcut(), &node("A"), &node("A"), 10).unwrap();
    assert_eq!(path.length(), 0);
    assert_eq!(ids(&path), vec!["A"]);
}

#[test]
fn all_shortest_paths_returns_every_minimum_length_tie() {
    // Two distinct 2-hop routes A->B->D and A->C->D.
    let adjacency = graph(&[
        ("A", "B", "LINK", None),
        ("A", "C", "LINK", None),
        ("B", "D", "LINK", None),
        ("C", "D", "LINK", None),
        ("A", "E", "LINK", None),
        ("E", "F", "LINK", None),
        ("F", "D", "LINK", None),
    ]);

    let paths = all_shortest_paths(&adjacency, &node("A"), &node("D"), 10, 100);
    assert_eq!(paths.len(), 2, "both 2-hop routes tie");
    assert!(paths.iter().all(|p| p.length() == 2));

    let mut routes: Vec<Vec<String>> = paths.iter().map(ids).collect();
    routes.sort();
    assert_eq!(routes, vec![vec!["A", "B", "D"], vec!["A", "C", "D"],]);
}

#[test]
fn all_shortest_paths_respects_the_result_cap() {
    let adjacency = graph(&[
        ("A", "B", "LINK", None),
        ("A", "C", "LINK", None),
        ("B", "D", "LINK", None),
        ("C", "D", "LINK", None),
    ]);
    assert_eq!(
        all_shortest_paths(&adjacency, &node("A"), &node("D"), 10, 1).len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Weighted / ANY CHEAPEST
// ---------------------------------------------------------------------------

/// One expensive direct hop versus a cheap three-hop detour.
///
/// `Apron -> Gate` costs 100 in one hop; `Apron -> R1 -> R2 -> Gate` costs 3
/// in three hops. Hop count and cost therefore disagree, which is the whole
/// point of the airport routing case.
fn airport_apron() -> GraphAdjacency {
    graph(&[
        ("Apron", "Gate", "Taxiway", Some(100.0)),
        ("Apron", "R1", "Taxiway", Some(1.0)),
        ("R1", "R2", "Taxiway", Some(1.0)),
        ("R2", "Gate", "Taxiway", Some(1.0)),
    ])
}

#[test]
fn cheapest_route_is_not_the_fewest_hops() {
    let adjacency = airport_apron();

    // ANY SHORTEST takes the single expensive hop...
    let hops = shortest_path(&adjacency, &node("Apron"), &node("Gate"), 10).unwrap();
    assert_eq!(hops.length(), 1);
    assert_eq!(ids(&hops), vec!["Apron", "Gate"]);

    // ...ANY CHEAPEST takes the three cheap ones.
    let cheap = cheapest_path(
        &adjacency,
        &node("Apron"),
        &node("Gate"),
        &CostSpec::EdgeWeight,
    )
    .expect("every edge is weighted")
    .expect("Gate is reachable");
    assert_eq!(cheap.length(), 3);
    assert_eq!(ids(&cheap), vec!["Apron", "R1", "R2", "Gate"]);
    assert_eq!(cheap.total_weight(), Some(3.0));
}

#[test]
fn cheapest_path_keeps_the_edge_weights_on_the_returned_path() {
    let cheap = cheapest_path(
        &airport_apron(),
        &node("Apron"),
        &node("Gate"),
        &CostSpec::EdgeWeight,
    )
    .unwrap()
    .unwrap();
    assert!(cheap.edges.iter().all(|e| e.weight == Some(1.0)));
}

#[test]
fn cost_one_is_equivalent_to_any_shortest() {
    let spec = CostSpec::constant(1.0).unwrap();
    let cheap = cheapest_path(&airport_apron(), &node("Apron"), &node("Gate"), &spec)
        .unwrap()
        .unwrap();
    assert_eq!(cheap.length(), 1, "constant cost degenerates to hop count");
}

#[test]
fn a_traversed_edge_without_a_weight_is_an_error_not_a_default() {
    // The only route out of A is unweighted. A `unwrap_or(1.0)` implementation
    // would happily answer with a hop count instead.
    let adjacency = graph(&[("A", "B", "LINK", None), ("B", "C", "LINK", Some(1.0))]);

    let err = cheapest_path(&adjacency, &node("A"), &node("C"), &CostSpec::EdgeWeight)
        .expect_err("an unweighted edge must fail loudly");
    match err {
        CostError::MissingWeight {
            ref relation_type,
            ref source_node,
            ref target_node,
        } => {
            assert_eq!(relation_type, "LINK");
            assert_eq!(source_node, "ws:A");
            assert_eq!(target_node, "ws:B");
        }
        other => panic!("expected MissingWeight, got {other:?}"),
    }
}

#[test]
fn a_non_positive_weight_is_an_error() {
    let adjacency = graph(&[("A", "B", "LINK", Some(-1.0))]);
    let err = cheapest_path(&adjacency, &node("A"), &node("B"), &CostSpec::EdgeWeight)
        .expect_err("Dijkstra is undefined for non-positive edges");
    assert!(matches!(err, CostError::InvalidWeight { .. }));
}

#[test]
fn an_unreachable_target_is_none_not_an_error() {
    let adjacency = graph(&[("A", "B", "LINK", Some(1.0)), ("C", "D", "LINK", Some(1.0))]);
    let result = cheapest_path(&adjacency, &node("A"), &node("D"), &CostSpec::EdgeWeight).unwrap();
    assert!(result.is_none());
}

#[test]
fn cheapest_path_prefers_a_cheaper_parallel_edge_between_the_same_pair() {
    // Two relation types between the same pair, different weights. An
    // implementation keyed on (source, target) alone would lose one of them.
    let adjacency = graph(&[
        ("A", "B", "toll", Some(50.0)),
        ("A", "B", "free", Some(1.0)),
    ]);
    let cheap = cheapest_path(&adjacency, &node("A"), &node("B"), &CostSpec::EdgeWeight)
        .unwrap()
        .unwrap();
    assert_eq!(cheap.edges[0].relation_type, "free");
    assert_eq!(cheap.total_weight(), Some(1.0));
}

// ---------------------------------------------------------------------------
// Yen's K shortest paths
// ---------------------------------------------------------------------------

#[test]
fn k_shortest_paths_returns_k_routes_in_cost_order() {
    let adjacency = graph(&[
        ("A", "B", "LINK", None),
        ("A", "C", "LINK", None),
        ("B", "D", "LINK", None),
        ("B", "C", "LINK", None),
        ("C", "D", "LINK", None),
    ]);

    let paths = k_shortest_paths(&adjacency, &node("A"), &node("D"), 3, |_, _, _| 1.0);
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0].length(), 2);
    assert_eq!(paths[1].length(), 2);
    assert_eq!(paths[2].length(), 3);
}

#[test]
fn k_shortest_paths_orders_by_cost_not_by_hop_count() {
    let adjacency = airport_apron();

    // Uniform cost: the single expensive hop is the cheapest route.
    let uniform = k_shortest_paths(&adjacency, &node("Apron"), &node("Gate"), 2, |_, _, _| 1.0);
    assert_eq!(uniform[0].length(), 1, "fixture: one hop is the shortest");

    // Real cost: Apron->Gate direct is 100, every other taxiway hop is 1.
    let priced = k_shortest_paths(
        &adjacency,
        &node("Apron"),
        &node("Gate"),
        2,
        |src, tgt, _| {
            if src.1 == "Apron" && tgt.1 == "Gate" {
                100.0
            } else {
                1.0
            }
        },
    );
    assert_eq!(priced[0].length(), 3, "the cheap detour ranks first");
    assert_eq!(ids(&priced[0]), vec!["Apron", "R1", "R2", "Gate"]);
    assert_eq!(
        priced[1].length(),
        1,
        "the expensive direct hop ranks second"
    );
}
