use super::*;

fn create_test_graph() -> GraphProjection {
    let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("B".to_string(), "C".to_string()),
        ("A".to_string(), "C".to_string()),
    ];
    GraphProjection::from_parts(nodes, edges)
}

#[test]
fn test_from_parts_basic() {
    let proj = create_test_graph();
    assert_eq!(proj.node_count(), 3);
    assert_eq!(proj.edge_count(), 3);
    assert!(proj.get_id("A").is_some());
    assert!(proj.get_id("D").is_none());
}

#[test]
fn test_from_parts_weighted() {
    let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let edges = vec![
        ("A".to_string(), "B".to_string(), 2.5),
        ("B".to_string(), "C".to_string(), 1.0),
        ("A".to_string(), "C".to_string(), 5.0),
    ];
    let proj = GraphProjection::from_parts_weighted(nodes, edges);

    assert_eq!(proj.node_count(), 3);
    assert_eq!(proj.edge_count(), 3);
    assert!(proj.has_weights());

    // Weights should be accessible
    let weights = proj.edge_weights().unwrap();
    assert_eq!(weights.len(), 3);

    // Edges are sorted by (source, target) in CSR ordering
    // With A=0, B=1, C=2: edges sorted as (0,1), (0,2), (1,2)
    // So weights should be [2.5, 5.0, 1.0]
    let a_idx = proj.get_id("A").unwrap();
    let b_idx = proj.get_id("B").unwrap();
    let c_idx = proj.get_id("C").unwrap();

    // Verify weight ordering matches CSR edge ordering
    assert_eq!(a_idx, 0);
    assert_eq!(b_idx, 1);
    assert_eq!(c_idx, 2);
    assert_eq!(proj.edge_weight(0), 2.5); // A->B
    assert_eq!(proj.edge_weight(1), 5.0); // A->C
    assert_eq!(proj.edge_weight(2), 1.0); // B->C
}

#[test]
fn test_unweighted_edge_weight_returns_default() {
    let proj = create_test_graph();
    assert!(!proj.has_weights());
    assert_eq!(proj.edge_weight(0), 1.0);
    assert_eq!(proj.edge_weight(999), 1.0);
}

#[test]
fn test_backward_graph_not_built_by_default() {
    let proj = create_test_graph();
    assert!(proj.backward_graph().is_none());
}

#[test]
fn test_ensure_backward_graph() {
    let mut proj = create_test_graph();
    proj.ensure_backward_graph();

    let bg = proj.backward_graph().unwrap();

    // Forward: A->B, A->C, B->C
    // Backward: B->A, C->A, C->B
    let a = proj.get_id("A").unwrap();
    let b = proj.get_id("B").unwrap();
    let c = proj.get_id("C").unwrap();

    // A has no in-neighbors in backward graph
    let a_in: Vec<u32> = if (a as usize) < bg.node_count() {
        bg.neighbors_slice(a).to_vec()
    } else {
        vec![]
    };
    assert!(a_in.is_empty(), "A should have no in-neighbors");

    // B has in-neighbor A
    let b_in: Vec<u32> = bg.neighbors_slice(b).to_vec();
    assert_eq!(b_in, vec![a], "B should have in-neighbor A");

    // C has in-neighbors A and B
    let mut c_in: Vec<u32> = bg.neighbors_slice(c).to_vec();
    c_in.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(c_in, expected, "C should have in-neighbors A and B");
}

#[test]
fn test_ensure_backward_graph_idempotent() {
    let mut proj = create_test_graph();
    proj.ensure_backward_graph();
    let ptr1 = proj.backward_graph().unwrap() as *const _;
    proj.ensure_backward_graph();
    let ptr2 = proj.backward_graph().unwrap() as *const _;
    assert_eq!(ptr1, ptr2, "Second call should not rebuild");
}

#[test]
fn test_empty_projection() {
    let proj = GraphProjection::from_parts(vec![], vec![]);
    assert_eq!(proj.node_count(), 0);
    assert_eq!(proj.edge_count(), 0);
    assert!(!proj.has_weights());
}

#[test]
fn test_weighted_empty() {
    let proj = GraphProjection::from_parts_weighted(vec![], vec![]);
    assert_eq!(proj.node_count(), 0);
    assert_eq!(proj.edge_count(), 0);
    // Empty weighted graph has Some([]) weights
    assert!(proj.has_weights());
}

#[test]
fn test_backward_graph_empty() {
    let mut proj = GraphProjection::from_parts(vec!["A".to_string()], vec![]);
    proj.ensure_backward_graph();
    assert!(proj.backward_graph().is_some());
}
