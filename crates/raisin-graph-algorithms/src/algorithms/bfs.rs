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
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Level-synchronous parallel Breadth-First Search.
///
/// Computes shortest-hop distances from `source` to all reachable nodes.
/// Uses a frontier-based approach with parallel neighbor expansion and
/// atomic compare-exchange for thread-safe distance updates.
///
/// # Arguments
/// * `projection` - The graph projection to traverse
/// * `source` - The string ID of the source node
///
/// # Returns
/// A `HashMap<String, u64>` mapping each node to its BFS distance.
/// The source node has distance 0. Unreachable nodes have distance `u64::MAX`.
/// If the source is not in the graph, returns an empty map.
///
/// # Complexity
/// O(V + E) work, O(diameter) depth with parallel frontier expansion.
pub fn bfs(projection: &GraphProjection, source: &str) -> HashMap<String, u64> {
    let node_count = projection.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    let source_id = match projection.get_id(source) {
        Some(id) => id,
        None => return HashMap::new(),
    };

    let graph = projection.graph();

    // Initialize distances to u64::MAX (unreachable), source to 0
    let distances: Vec<AtomicU64> = (0..node_count).map(|_| AtomicU64::new(u64::MAX)).collect();
    distances[source_id as usize].store(0, Ordering::Relaxed);

    let mut frontier: Vec<u32> = vec![source_id];
    let mut level: u64 = 0;

    while !frontier.is_empty() {
        level += 1;
        let next_level = level;

        // Parallel frontier expansion with thread-local next-frontier collection
        let next_frontier: Vec<u32> = frontier
            .par_iter()
            .flat_map(|&u| {
                let mut local_next = Vec::new();
                if (u as usize) < graph.node_count() {
                    for &v in graph.neighbors_slice(u) {
                        // Attempt to claim v at next_level (only succeeds if v is unvisited)
                        if distances[v as usize]
                            .compare_exchange(
                                u64::MAX,
                                next_level,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                        {
                            local_next.push(v);
                        }
                    }
                }
                local_next
            })
            .collect();

        frontier = next_frontier;
    }

    // Map back to String IDs, including all nodes (unreachable ones keep u64::MAX)
    let mut result = HashMap::with_capacity(node_count);
    for (i, d) in distances.iter().enumerate() {
        let dist = d.load(Ordering::Relaxed);
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), dist);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_bfs_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let result = bfs(&projection, "A");
        assert!(result.is_empty());
    }

    #[test]
    fn test_bfs_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "A");
        assert_eq!(result.len(), 1);
        assert_eq!(result["A"], 0);
    }

    #[test]
    fn test_bfs_line_graph() {
        // A -> B -> C -> D
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "A");
        assert_eq!(result["A"], 0);
        assert_eq!(result["B"], 1);
        assert_eq!(result["C"], 2);
        assert_eq!(result["D"], 3);
    }

    #[test]
    fn test_bfs_star_graph() {
        // Center -> S1, Center -> S2, Center -> S3
        let nodes = vec![
            "Center".to_string(),
            "S1".to_string(),
            "S2".to_string(),
            "S3".to_string(),
        ];
        let edges = vec![
            ("Center".to_string(), "S1".to_string()),
            ("Center".to_string(), "S2".to_string()),
            ("Center".to_string(), "S3".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "Center");
        assert_eq!(result["Center"], 0);
        assert_eq!(result["S1"], 1);
        assert_eq!(result["S2"], 1);
        assert_eq!(result["S3"], 1);
    }

    #[test]
    fn test_bfs_disconnected() {
        // A -> B, C (isolated)
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "A");
        assert_eq!(result["A"], 0);
        assert_eq!(result["B"], 1);
        // C is unreachable, should have u64::MAX per LDBC spec
        assert_eq!(result["C"], u64::MAX);
    }

    #[test]
    fn test_bfs_cycle() {
        // A -> B -> C -> A
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "A");
        assert_eq!(result["A"], 0);
        assert_eq!(result["B"], 1);
        assert_eq!(result["C"], 2);
    }

    #[test]
    fn test_bfs_source_not_in_graph() {
        let nodes = vec!["A".to_string(), "B".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())];
        let projection = GraphProjection::from_parts(nodes, edges);

        let result = bfs(&projection, "Z");
        assert!(result.is_empty());
    }
}
