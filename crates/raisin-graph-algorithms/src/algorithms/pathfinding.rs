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
            // e is EdgeReference. Csr edges don't store weights directly in this projection yet.
            // We use the callback to get cost.
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

/// Yen's K-Shortest Paths
///
/// Finds the K shortest paths between two nodes using Yen's algorithm.
/// Returns a vector of (cost, path_nodes).
pub fn k_shortest_paths<F>(
    projection: &GraphProjection,
    start_node: &str,
    end_node: &str,
    k: usize,
    edge_cost: F,
) -> Option<Vec<(f64, Vec<String>)>>
where
    F: Fn(u32, u32) -> f64,
{
    let graph = projection.graph();
    let start_idx = projection.get_id(start_node)?;
    let end_idx = projection.get_id(end_node)?;

    // 1. Find shortest path (k=0)
    let mut paths: Vec<(f64, Vec<u32>)> = Vec::new();

    // Helper for Dijkstra/A*
    let find_path = |src: u32,
                     dst: u32,
                     excluded_edges: &std::collections::HashSet<(u32, u32)>,
                     excluded_nodes: &std::collections::HashSet<u32>|
     -> Option<(f64, Vec<u32>)> {
        petgraph::algo::astar(
            graph,
            src,
            |finish| finish == dst,
            |e| {
                use petgraph::visit::EdgeRef;
                let u = e.source();
                let v = e.target();
                if excluded_edges.contains(&(u, v))
                    || excluded_nodes.contains(&u)
                    || excluded_nodes.contains(&v)
                {
                    f64::INFINITY
                } else {
                    edge_cost(u, v)
                }
            },
            |_| 0.0,
        )
    };

    if let Some((cost, path)) = find_path(
        start_idx,
        end_idx,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    ) {
        paths.push((cost, path));
    } else {
        return None;
    }

    let mut potential_paths = std::collections::BinaryHeap::new();

    for i in 1..k {
        if paths.len() < i {
            break;
        }
        let prev_path = &paths[i - 1].1;

        for j in 0..prev_path.len() - 1 {
            let spur_node = prev_path[j];
            let root_path = &prev_path[0..=j];

            let mut excluded_edges = std::collections::HashSet::new();
            let mut excluded_nodes = std::collections::HashSet::new();

            // Exclude edges used in existing paths with same root
            for p_tuple in &paths {
                let p = &p_tuple.1;
                if p.len() > j && &p[0..=j] == root_path {
                    excluded_edges.insert((p[j], p[j + 1]));
                }
            }

            // Exclude nodes in root path (except spur node) to ensure loopless
            for &n in &root_path[0..j] {
                excluded_nodes.insert(n);
            }

            if let Some((_spur_cost, spur_path)) =
                find_path(spur_node, end_idx, &excluded_edges, &excluded_nodes)
            {
                // Total path = root_path + spur_path[1..]
                let mut total_path = root_path.to_vec();
                total_path.extend_from_slice(&spur_path[1..]);

                // Recalculate total cost
                let mut total_cost = 0.0;
                for w in 0..total_path.len() - 1 {
                    total_cost += edge_cost(total_path[w], total_path[w + 1]);
                }

                potential_paths.push(OrderedPath {
                    cost: total_cost,
                    path: total_path,
                });
            }
        }

        // Add best potential path that isn't already in paths
        while let Some(best) = potential_paths.pop() {
            let exists = paths.iter().any(|(_, p)| p == &best.path);
            if !exists {
                paths.push((best.cost, best.path));
                break;
            }
        }
    }

    // Convert to strings
    let result = paths
        .into_iter()
        .map(|(cost, path_indices)| {
            let path_ids = path_indices
                .into_iter()
                .filter_map(|idx| projection.get_node_id(idx).cloned())
                .collect();
            (cost, path_ids)
        })
        .collect();

    Some(result)
}

#[derive(PartialEq)]
struct OrderedPath {
    cost: f64,
    path: Vec<u32>,
}

impl Eq for OrderedPath {}

impl Ord for OrderedPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap behavior for cost
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for OrderedPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
    fn test_k_shortest_paths() {
        let projection = create_test_graph();
        // Paths A->E:
        // 1. A->B->D->E (1+1+1 = 3)
        // 2. A->C->D->E (2+1+1 = 4)
        // 3. A->C->E (2+3 = 5)

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

        // Check costs
        assert!((paths[0].0 - 3.0).abs() < 1e-6);
        assert!((paths[1].0 - 4.0).abs() < 1e-6);
        assert!((paths[2].0 - 5.0).abs() < 1e-6);

        // Check paths
        assert_eq!(paths[0].1, vec!["A", "B", "D", "E"]);
        assert_eq!(paths[1].1, vec!["A", "C", "D", "E"]);
        assert_eq!(paths[2].1, vec!["A", "C", "E"]);
    }

    // ==================== A* Tests ====================

    #[test]
    fn test_astar_shortest_path() {
        let projection = create_test_graph();
        // Test graph has: A->B, A->C, B->D, C->D, C->E, D->E
        // Shortest path A->E: A->C->E (2 edges)

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
        // A -> B, C is isolated
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
        // A* with heuristic = 0 should behave like Dijkstra
        let projection = create_test_graph();

        let result = astar(&projection, "A", "E", |_, _| 1.0, |_| 0.0);

        assert!(result.is_some());
        let (cost, _path) = result.unwrap();
        // Shortest path A->C->E = 2 edges
        assert_eq!(cost, 2.0);
    }

    #[test]
    fn test_astar_weighted_edges() {
        let projection = create_test_graph();

        // Custom weights making A->C->E cheaper than A->B->D->E
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

        // A->C->E = 1 + 1 = 2
        assert_eq!(cost, 2.0, "Should find cheaper weighted path");
        assert_eq!(path, vec!["A", "C", "E"]);
    }

    // ==================== K-Shortest Paths Additional Tests ====================

    #[test]
    fn test_k_shortest_single_path() {
        let projection = create_test_graph();

        let result = k_shortest_paths(&projection, "A", "E", 1, |_, _| 1.0);

        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths.len(), 1, "Should return exactly 1 path when k=1");
        // Shortest path A->C->E = 2 edges
        assert_eq!(paths[0].0, 2.0, "Shortest path cost should be 2");
    }

    #[test]
    fn test_k_shortest_no_path() {
        // A -> B, C is isolated
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
        // Simple graph with only 2 paths from A to C
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        // Ask for 5 paths when only 2 exist
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

        // Verify paths are in ascending cost order
        for i in 1..paths.len() {
            assert!(
                paths[i].0 >= paths[i - 1].0,
                "Paths should be ordered by cost: {} >= {}",
                paths[i].0,
                paths[i - 1].0
            );
        }
    }
}
