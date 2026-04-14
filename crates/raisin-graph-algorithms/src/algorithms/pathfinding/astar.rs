// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use crate::projection::GraphProjection;

/// A* Pathfinding
///
/// Finds the shortest path between two nodes using A* algorithm.
/// Requires a heuristic function and edge weights.
/// Returns a vector of NodeIDs representing the path.
pub fn astar<F, H>(
    projection: &GraphProjection,
    start_node: &str,
    end_node: &str,
    edge_cost: F,
    heuristic: H,
) -> Option<(f64, Vec<String>)>
where
    F: Fn(u32, u32) -> f64,
    H: Fn(u32) -> f64,
{
    let graph = projection.graph();

    // Map string IDs to internal indices
    let start_idx = projection.get_id(start_node)?;
    let end_idx = projection.get_id(end_node)?;

    // Use petgraph's astar
    let result = petgraph::algo::astar(
        graph,
        start_idx,
        |finish| finish == end_idx,
        |e| {
            use petgraph::visit::EdgeRef;
            edge_cost(e.source(), e.target())
        },
        heuristic,
    );

    match result {
        Some((cost, path_indices)) => {
            let path_ids: Vec<String> = path_indices
                .into_iter()
                .filter_map(|idx| projection.get_node_id(idx).cloned())
                .collect();
            Some((cost, path_ids))
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_astar_shortest_path() {
        let projection = create_test_graph();

        let result = astar(&projection, "A", "E", |_, _| 1.0, |_| 0.0);

        assert!(result.is_some());
        let (cost, path) = result.unwrap();

        assert_eq!(cost, 2.0, "Shortest path A->C->E cost should be 2");
        assert_eq!(path.first(), Some(&"A".to_string()));
        assert_eq!(path.last(), Some(&"E".to_string()));
        assert_eq!(path.len(), 3, "Path A->C->E should have 3 nodes");
    }

    #[test]
    fn test_astar_no_path() {
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = astar(&projection, "A", "C", |_, _| 1.0, |_| 0.0);

        assert!(result.is_none(), "Should return None when no path exists");
    }

    #[test]
    fn test_astar_start_equals_end() {
        let projection = create_test_graph();

        let result = astar(&projection, "A", "A", |_, _| 1.0, |_| 0.0);

        assert!(result.is_some());
        let (cost, path) = result.unwrap();

        assert_eq!(cost, 0.0, "Path to self should have cost 0");
        assert_eq!(path, vec!["A"], "Path to self should contain only start");
    }

    #[test]
    fn test_astar_nonexistent_start() {
        let projection = create_test_graph();

        let result = astar(&projection, "NONEXISTENT", "E", |_, _| 1.0, |_| 0.0);

        assert!(
            result.is_none(),
            "Should return None for nonexistent start node"
        );
    }

    #[test]
    fn test_astar_nonexistent_end() {
        let projection = create_test_graph();

        let result = astar(&projection, "A", "NONEXISTENT", |_, _| 1.0, |_| 0.0);

        assert!(
            result.is_none(),
            "Should return None for nonexistent end node"
        );
    }

    #[test]
    fn test_astar_dijkstra_mode() {
        let projection = create_test_graph();

        let result = astar(&projection, "A", "E", |_, _| 1.0, |_| 0.0);

        assert!(result.is_some());
        let (cost, _path) = result.unwrap();
        assert_eq!(cost, 2.0);
    }

    #[test]
    fn test_astar_weighted_edges() {
        let projection = create_test_graph();

        let edge_weights: HashMap<(String, String), f64> = vec![
            (("A".to_string(), "B".to_string()), 10.0),
            (("A".to_string(), "C".to_string()), 1.0),
            (("B".to_string(), "D".to_string()), 1.0),
            (("C".to_string(), "D".to_string()), 1.0),
            (("C".to_string(), "E".to_string()), 1.0),
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

        let result = astar(&projection, "A", "E", cost_fn, |_| 0.0);

        assert!(result.is_some());
        let (cost, path) = result.unwrap();

        assert_eq!(cost, 2.0, "Should find cheaper weighted path");
        assert_eq!(path, vec!["A", "C", "E"]);
    }
}
