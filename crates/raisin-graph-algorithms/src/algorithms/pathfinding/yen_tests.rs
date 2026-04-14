use super::k_shortest_paths;
use crate::projection::GraphProjection;
use std::collections::HashMap;

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
fn test_k_shortest_paths() {
    let projection = create_test_graph();

    let edge_weights: HashMap<(String, String), f64> = vec![
        (("A".to_string(), "B".to_string()), 1.0),
        (("A".to_string(), "C".to_string()), 2.0),
        (("B".to_string(), "D".to_string()), 1.0),
        (("C".to_string(), "D".to_string()), 1.0),
        (("C".to_string(), "E".to_string()), 3.0),
        (("D".to_string(), "E".to_string()), 1.0),
    ]
    .into_iter()
    .collect();

    let cost_fn = |u: u32, v: u32| {
        let u_id = projection.get_node_id(u).unwrap();
        let v_id = projection.get_node_id(v).unwrap();
        *edge_weights
            .get(&(u_id.clone(), v_id.clone()))
            .unwrap_or(&1.0)
    };

    let paths = k_shortest_paths(&projection, "A", "E", 3, cost_fn).unwrap();

    assert_eq!(paths.len(), 3);
    assert!((paths[0].0 - 3.0).abs() < 1e-6);
    assert!((paths[1].0 - 4.0).abs() < 1e-6);
    assert!((paths[2].0 - 5.0).abs() < 1e-6);

    assert_eq!(paths[0].1, vec!["A", "B", "D", "E"]);
    assert_eq!(paths[1].1, vec!["A", "C", "D", "E"]);
    assert_eq!(paths[2].1, vec!["A", "C", "E"]);
}

#[test]
fn test_k_shortest_single_path() {
    let projection = create_test_graph();

    let result = k_shortest_paths(&projection, "A", "E", 1, |_, _| 1.0);

    assert!(result.is_some());
    let paths = result.unwrap();
    assert_eq!(paths.len(), 1, "Should return exactly 1 path when k=1");
    assert_eq!(paths[0].0, 2.0, "Shortest path cost should be 2");
}

#[test]
fn test_k_shortest_no_path() {
    let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let edges = vec![("A".to_string(), "B".to_string())];
    let projection = GraphProjection::from_parts(nodes, edges);

    let result = k_shortest_paths(&projection, "A", "C", 3, |_, _| 1.0);

    assert!(result.is_none(), "Should return None when no path exists");
}

#[test]
fn test_k_shortest_start_equals_end() {
    let projection = create_test_graph();

    let result = k_shortest_paths(&projection, "A", "A", 3, |_, _| 1.0);

    assert!(result.is_some());
    let paths = result.unwrap();
    assert_eq!(paths.len(), 1, "Path to self should return 1 path");
    assert_eq!(paths[0].0, 0.0, "Path to self should have cost 0");
    assert_eq!(paths[0].1, vec!["A"], "Path should contain only start");
}

#[test]
fn test_k_shortest_fewer_than_k_paths() {
    let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let edges = vec![
        ("A".to_string(), "B".to_string()),
        ("A".to_string(), "C".to_string()),
        ("B".to_string(), "C".to_string()),
    ];
    let projection = GraphProjection::from_parts(nodes, edges);

    let result = k_shortest_paths(&projection, "A", "C", 5, |_, _| 1.0);

    assert!(result.is_some());
    let paths = result.unwrap();
    assert!(
        paths.len() <= 2,
        "Should return at most 2 paths (A->C and A->B->C)"
    );
}

#[test]
fn test_k_shortest_nonexistent_nodes() {
    let projection = create_test_graph();

    let result1 = k_shortest_paths(&projection, "NONEXISTENT", "E", 3, |_, _| 1.0);
    assert!(
        result1.is_none(),
        "Should return None for nonexistent start"
    );

    let result2 = k_shortest_paths(&projection, "A", "NONEXISTENT", 3, |_, _| 1.0);
    assert!(result2.is_none(), "Should return None for nonexistent end");
}

#[test]
fn test_k_shortest_paths_ordering() {
    let projection = create_test_graph();

    let edge_weights: HashMap<(String, String), f64> = vec![
        (("A".to_string(), "B".to_string()), 1.0),
        (("A".to_string(), "C".to_string()), 2.0),
        (("B".to_string(), "D".to_string()), 1.0),
        (("C".to_string(), "D".to_string()), 1.0),
        (("C".to_string(), "E".to_string()), 3.0),
        (("D".to_string(), "E".to_string()), 1.0),
    ]
    .into_iter()
    .collect();

    let cost_fn = |u: u32, v: u32| {
        let u_id = projection.get_node_id(u).unwrap();
        let v_id = projection.get_node_id(v).unwrap();
        *edge_weights
            .get(&(u_id.clone(), v_id.clone()))
            .unwrap_or(&f64::INFINITY)
    };

    let paths = k_shortest_paths(&projection, "A", "E", 3, cost_fn).unwrap();

    for i in 1..paths.len() {
        assert!(
            paths[i].0 >= paths[i - 1].0,
            "Paths should be ordered by cost: {} >= {}",
            paths[i].0,
            paths[i - 1].0
        );
    }
}
