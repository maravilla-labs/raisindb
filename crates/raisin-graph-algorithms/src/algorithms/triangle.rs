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

/// Raw per-node triangle count (indexed by integer node ID).
///
/// Counts the number of triangles each node participates in, treating the
/// graph as undirected. Returns a `Vec<usize>` where `result[i]` is the
/// triangle count for node with integer ID `i`.
///
/// This is the internal workhorse used by both `triangle_count` and `lcc`.
pub(crate) fn triangle_count_raw(projection: &GraphProjection) -> Vec<usize> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    // Build undirected adjacency from directed CSR edges
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); node_count];

    for u in 0..node_count {
        if u < graph.node_count() {
            for &v in graph.neighbors_slice(u as u32) {
                if u != v as usize {
                    adj[u].push(v);
                    adj[v as usize].push(u as u32);
                }
            }
        }
    }

    // Sort and deduplicate neighbors
    adj.par_iter_mut().for_each(|neighbors| {
        neighbors.sort_unstable();
        neighbors.dedup();
    });

    let mut counts = vec![0; node_count];

    // For each node u, count pairs of neighbors (v, w) that are connected.
    // Each triangle (u, v, w) is counted twice per node: once via v finding w,
    // once via w finding v. Divide by 2 for the correct count.
    counts.par_iter_mut().enumerate().for_each(|(u, count)| {
        let neighbors = &adj[u];
        let mut local_count = 0;

        for &v in neighbors {
            let v_neighbors = &adj[v as usize];

            // Sorted intersection to count common neighbors
            let mut i = 0;
            let mut j = 0;
            while i < neighbors.len() && j < v_neighbors.len() {
                let n1 = neighbors[i];
                let n2 = v_neighbors[j];
                if n1 == n2 {
                    local_count += 1;
                    i += 1;
                    j += 1;
                } else if n1 < n2 {
                    i += 1;
                } else {
                    j += 1;
                }
            }
        }
        *count = local_count / 2;
    });

    counts
}

/// Triangle Count
///
/// Counts the number of triangles (cycles of length 3) for each node.
/// Returns a map of NodeID -> Count.
/// Note: This implementation treats the graph as undirected.
pub fn triangle_count(projection: &GraphProjection) -> HashMap<String, usize> {
    let counts = triangle_count_raw(projection);
    let node_count = projection.node_count();

    let mut result = HashMap::with_capacity(node_count);
    for (i, &c) in counts.iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_triangle_count() {
        // Triangle A-B-C
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let counts = triangle_count(&projection);
        assert_eq!(counts["A"], 1);
        assert_eq!(counts["B"], 1);
        assert_eq!(counts["C"], 1);
    }

    // ==================== Triangle Count Additional Tests ====================

    #[test]
    fn test_triangle_count_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let counts = triangle_count(&projection);
        assert!(
            counts.is_empty(),
            "Empty graph should return empty triangle counts"
        );
    }

    #[test]
    fn test_triangle_count_no_triangles() {
        // Path graph: A -> B -> C (no triangles)
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let counts = triangle_count(&projection);

        assert_eq!(counts["A"], 0, "A should have 0 triangles");
        assert_eq!(counts["B"], 0, "B should have 0 triangles");
        assert_eq!(counts["C"], 0, "C should have 0 triangles");
    }

    #[test]
    fn test_triangle_count_multiple_triangles() {
        // Diamond: A-B-C-D with A-C and B-D diagonals
        // Forms 4 triangles
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
            ("D".to_string(), "A".to_string()),
            ("A".to_string(), "C".to_string()), // diagonal
            ("B".to_string(), "D".to_string()), // diagonal
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let counts = triangle_count(&projection);

        // Each node is part of 2 triangles in a K4-like structure
        // Triangles: ABC, ACD, ABD, BCD
        assert!(
            counts["A"] >= 1,
            "A should participate in at least 1 triangle"
        );
        assert!(
            counts["B"] >= 1,
            "B should participate in at least 1 triangle"
        );
    }

    #[test]
    fn test_triangle_count_shared_node() {
        // Node B is shared by two triangles: A-B-C and B-C-D
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            // Triangle 1: A-B-C
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
            // Triangle 2: B-C-D
            ("C".to_string(), "D".to_string()),
            ("D".to_string(), "B".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let counts = triangle_count(&projection);

        assert_eq!(counts["A"], 1, "A is in 1 triangle");
        assert_eq!(counts["B"], 2, "B is shared by 2 triangles");
        assert_eq!(counts["C"], 2, "C is shared by 2 triangles");
        assert_eq!(counts["D"], 1, "D is in 1 triangle");
    }

    #[test]
    fn test_triangle_count_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let counts = triangle_count(&projection);

        assert_eq!(counts["A"], 0, "Single node cannot form triangles");
    }
}
