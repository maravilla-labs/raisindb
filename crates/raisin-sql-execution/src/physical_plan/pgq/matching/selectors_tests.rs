//! Unit tests for [`super::selectors`].
//!
//! Split out to keep the implementation file under the 300-line limit; it is
//! `#[path]`-included so the tests keep access to private items.

use super::*;
use crate::physical_plan::graph_algo::PathNode;

fn chain(ids: &[&str], weights: &[f32]) -> GraphPath {
    let mut path = GraphPath::start(PathNode::new(ids[0], "ws", "T"));
    for (i, id) in ids.iter().enumerate().skip(1) {
        path.push(&GraphEdge::new(
            "ws",
            *id,
            "T",
            "r",
            weights.get(i - 1).copied(),
        ));
    }
    path
}

/// a->b->d (2 hops, weight 10) and a->c1->c2->d (3 hops, weight 3).
fn two_routes() -> Vec<GraphPath> {
    vec![
        chain(&["a", "b", "d"], &[5.0, 5.0]),
        chain(&["a", "c1", "c2", "d"], &[1.0, 1.0, 1.0]),
    ]
}

#[test]
fn no_selector_keeps_every_path() {
    assert_eq!(apply_selector(None, two_routes()).unwrap().len(), 2);
}

#[test]
fn any_shortest_takes_the_fewest_hops_not_the_cheapest() {
    let out = apply_selector(Some(PathSelector::AnyShortest), two_routes()).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].length(), 2);
}

#[test]
fn any_cheapest_takes_the_lowest_weight_not_the_fewest_hops() {
    let out = apply_selector(Some(PathSelector::AnyCheapest), two_routes()).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].length(), 3);
    assert_eq!(out[0].total_weight(), Some(3.0));
}

#[test]
fn any_cheapest_errors_rather_than_defaulting_a_missing_weight_to_one() {
    let unweighted = vec![chain(&["a", "b"], &[])];
    let err = apply_selector(Some(PathSelector::AnyCheapest), unweighted).unwrap_err();
    assert!(err.to_string().contains("has no weight"), "{err}");
}

#[test]
fn any_cheapest_rejects_non_positive_weights() {
    let bad = vec![chain(&["a", "b"], &[0.0])];
    let err = apply_selector(Some(PathSelector::AnyCheapest), bad).unwrap_err();
    assert!(err.to_string().contains("positive finite weight"), "{err}");
}

#[test]
fn all_shortest_keeps_every_minimum_hop_path_for_the_pair() {
    let mut paths = two_routes();
    paths.push(chain(&["a", "b2", "d"], &[1.0, 1.0]));
    let out = apply_selector(Some(PathSelector::AllShortest), paths).unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|p| p.length() == 2));
}

#[test]
fn selectors_group_by_endpoint_pair() {
    let mut paths = two_routes();
    paths.push(chain(&["a", "z"], &[1.0]));
    let out = apply_selector(Some(PathSelector::AnyShortest), paths).unwrap();
    // One winner for a->d and one for a->z.
    assert_eq!(out.len(), 2);
}

#[test]
fn trail_allows_revisiting_a_node_but_not_an_edge() {
    let path = chain(&["a", "b"], &[1.0]);
    let back = GraphEdge::new("ws", "a", "T", "r", Some(1.0));
    assert!(PathRestrictor::Trail.allows_extension(&path, &back));
    assert!(!PathRestrictor::Acyclic.allows_extension(&path, &back));
    assert!(PathRestrictor::Walk.allows_extension(&path, &back));
}

#[test]
fn the_default_restrictor_is_acyclic() {
    assert_eq!(PathSemantics::default().restrictor, PathRestrictor::Acyclic);
}

fn linear_graph() -> GraphAdjacency {
    let mut adj: GraphAdjacency = HashMap::new();
    adj.insert(
        ("ws".into(), "a".into()),
        vec![
            GraphEdge::new("ws", "b", "T", "r", Some(5.0)),
            GraphEdge::new("ws", "c1", "T", "r", Some(1.0)),
        ],
    );
    adj.insert(
        ("ws".into(), "b".into()),
        vec![GraphEdge::new("ws", "d", "T", "r", Some(5.0))],
    );
    adj.insert(
        ("ws".into(), "c1".into()),
        vec![GraphEdge::new("ws", "c2", "T", "r", Some(1.0))],
    );
    adj.insert(
        ("ws".into(), "c2".into()),
        vec![GraphEdge::new("ws", "d", "T", "r", Some(1.0))],
    );
    adj
}

#[test]
fn bound_endpoints_use_the_shared_algorithms() {
    let out = selector_over_bound_endpoints(
        PathSelector::AnyShortest,
        PathRestrictor::Acyclic,
        1,
        10,
        &linear_graph(),
        &("ws".into(), "a".into()),
        "Account",
        &("ws".into(), "d".into()),
        100,
    )
    .unwrap()
    .expect("fast path applies");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].length(), 2);
    assert_eq!(out[0].nodes[0].node_type, "Account");
}

#[test]
fn bound_endpoints_cheapest_prefers_weight_over_hops() {
    let out = selector_over_bound_endpoints(
        PathSelector::AnyCheapest,
        PathRestrictor::Acyclic,
        1,
        10,
        &linear_graph(),
        &("ws".into(), "a".into()),
        "T",
        &("ws".into(), "d".into()),
        100,
    )
    .unwrap()
    .expect("fast path applies");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].length(), 3);
}

#[test]
fn the_fast_path_declines_when_a_restrictor_or_lower_bound_makes_it_wrong() {
    for (restrictor, min) in [(PathRestrictor::Trail, 1), (PathRestrictor::Acyclic, 2)] {
        let out = selector_over_bound_endpoints(
            PathSelector::AnyShortest,
            restrictor,
            min,
            10,
            &linear_graph(),
            &("ws".into(), "a".into()),
            "T",
            &("ws".into(), "d".into()),
            100,
        )
        .unwrap();
        assert!(out.is_none(), "{restrictor:?} min={min} must enumerate");
    }
}
