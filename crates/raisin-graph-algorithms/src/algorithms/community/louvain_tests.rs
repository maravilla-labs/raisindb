use super::louvain;
use crate::projection::GraphProjection;

fn create_test_graph() -> GraphProjection {
    let nodes = vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
        "E".to_string(),
    ];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("A".to_string(), "C".to_string()),
        ("B".to_string(), "D".to_string()),
        ("C".to_string(), "D".to_string()),
        ("C".to_string(), "E".to_string()),
        ("D".to_string(), "E".to_string()),
    ];
    GraphProjection::from_parts(nodes, edges)
}

#[test]
fn test_louvain_empty_graph() {
    let projection = GraphProjection::from_parts(vec![], vec![]);
    let communities = louvain(&projection, 10, 1.0);
    assert!(
        communities.is_empty(),
        "Empty graph should return empty communities"
    );
}

#[test]
fn test_louvain_single_node() {
    let nodes = vec!["A".to_string()];
    let edges: Vec<(String, String)> = vec![];
    let projection = GraphProjection::from_parts(nodes, edges);

    let communities = louvain(&projection, 10, 1.0);

    assert_eq!(communities.len(), 1);
    assert!(
        communities.contains_key("A"),
        "Single node should be assigned a community"
    );
}

#[test]
fn test_louvain_all_nodes_assigned() {
    let projection = create_test_graph();
    let communities = louvain(&projection, 10, 1.0);

    assert_eq!(communities.len(), 5);
    assert!(communities.contains_key("A"));
    assert!(communities.contains_key("B"));
    assert!(communities.contains_key("C"));
    assert!(communities.contains_key("D"));
    assert!(communities.contains_key("E"));
}

#[test]
fn test_louvain_no_edges_separate_communities() {
    let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let edges: Vec<(String, String)> = vec![];
    let projection = GraphProjection::from_parts(nodes, edges);

    let communities = louvain(&projection, 10, 1.0);
    assert_eq!(communities.len(), 3);
}

#[test]
fn test_louvain_clique_same_community() {
    let nodes = vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("A".to_string(), "C".to_string()),
        ("A".to_string(), "D".to_string()),
        ("B".to_string(), "A".to_string()),
        ("B".to_string(), "C".to_string()),
        ("B".to_string(), "D".to_string()),
        ("C".to_string(), "A".to_string()),
        ("C".to_string(), "B".to_string()),
        ("C".to_string(), "D".to_string()),
        ("D".to_string(), "A".to_string()),
        ("D".to_string(), "B".to_string()),
        ("D".to_string(), "C".to_string()),
    ];
    let projection = GraphProjection::from_parts(nodes, edges);

    let communities = louvain(&projection, 20, 1.0);

    let comm_a = communities["A"];
    assert_eq!(
        communities["B"], comm_a,
        "Clique members should be in same community"
    );
    assert_eq!(
        communities["C"], comm_a,
        "Clique members should be in same community"
    );
    assert_eq!(
        communities["D"], comm_a,
        "Clique members should be in same community"
    );
}

#[test]
fn test_louvain_disconnected_different_communities() {
    let nodes = vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "X".to_string(),
        "Y".to_string(),
        "Z".to_string(),
    ];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("B".to_string(), "A".to_string()),
        ("A".to_string(), "C".to_string()),
        ("C".to_string(), "A".to_string()),
        ("B".to_string(), "C".to_string()),
        ("C".to_string(), "B".to_string()),
        ("X".to_string(), "Y".to_string()),
        ("Y".to_string(), "X".to_string()),
        ("X".to_string(), "Z".to_string()),
        ("Z".to_string(), "X".to_string()),
        ("Y".to_string(), "Z".to_string()),
        ("Z".to_string(), "Y".to_string()),
    ];
    let projection = GraphProjection::from_parts(nodes, edges);

    let communities = louvain(&projection, 20, 1.0);

    assert_eq!(communities["A"], communities["B"]);
    assert_eq!(communities["A"], communities["C"]);
    assert_eq!(communities["X"], communities["Y"]);
    assert_eq!(communities["X"], communities["Z"]);
    assert_ne!(
        communities["A"], communities["X"],
        "Disconnected cliques should be in different communities"
    );
}

#[test]
fn test_louvain_resolution_parameter() {
    let nodes = vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
        "E".to_string(),
        "F".to_string(),
    ];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("B".to_string(), "A".to_string()),
        ("A".to_string(), "C".to_string()),
        ("C".to_string(), "A".to_string()),
        ("B".to_string(), "C".to_string()),
        ("C".to_string(), "B".to_string()),
        ("D".to_string(), "E".to_string()),
        ("E".to_string(), "D".to_string()),
        ("D".to_string(), "F".to_string()),
        ("F".to_string(), "D".to_string()),
        ("E".to_string(), "F".to_string()),
        ("F".to_string(), "E".to_string()),
        ("C".to_string(), "D".to_string()),
        ("D".to_string(), "C".to_string()),
    ];
    let projection = GraphProjection::from_parts(nodes, edges);

    let low_res = louvain(&projection, 20, 0.5);
    let high_res = louvain(&projection, 20, 2.0);

    let low_res_unique: std::collections::HashSet<_> = low_res.values().collect();
    let high_res_unique: std::collections::HashSet<_> = high_res.values().collect();

    assert!(
        high_res_unique.len() >= low_res_unique.len(),
        "Higher resolution should produce more or equal communities: low={}, high={}",
        low_res_unique.len(),
        high_res_unique.len()
    );
}

#[test]
fn test_louvain_hierarchical_two_levels() {
    let nodes = vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
        "E".to_string(),
        "F".to_string(),
    ];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("B".to_string(), "A".to_string()),
        ("A".to_string(), "C".to_string()),
        ("C".to_string(), "A".to_string()),
        ("B".to_string(), "C".to_string()),
        ("C".to_string(), "B".to_string()),
        ("D".to_string(), "E".to_string()),
        ("E".to_string(), "D".to_string()),
        ("D".to_string(), "F".to_string()),
        ("F".to_string(), "D".to_string()),
        ("E".to_string(), "F".to_string()),
        ("F".to_string(), "E".to_string()),
        ("C".to_string(), "D".to_string()),
        ("D".to_string(), "C".to_string()),
    ];
    let projection = GraphProjection::from_parts(nodes, edges);

    let communities = louvain(&projection, 20, 1.0);

    assert_eq!(communities.len(), 6);
    assert_eq!(
        communities["A"], communities["B"],
        "A and B should be in the same community"
    );
    assert_eq!(
        communities["A"], communities["C"],
        "A and C should be in the same community"
    );
    assert_eq!(
        communities["D"], communities["E"],
        "D and E should be in the same community"
    );
    assert_eq!(
        communities["D"], communities["F"],
        "D and F should be in the same community"
    );
    assert_ne!(
        communities["A"], communities["D"],
        "Group {{A,B,C}} and group {{D,E,F}} should be in different communities"
    );

    let unique: std::collections::HashSet<_> = communities.values().collect();
    assert_eq!(
        unique.len(),
        2,
        "Should detect exactly 2 communities for two weakly-linked cliques"
    );
}
