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

use crate::algorithms::triangle::triangle_count_raw;
use crate::projection::GraphProjection;
use rayon::prelude::*;
use std::collections::HashMap;

/// Local Clustering Coefficient (LCC)
///
/// Computes the local clustering coefficient for each node in the graph,
/// treating edges as undirected.
///
/// The LCC of a node `v` measures how close its neighbors are to forming
/// a clique (complete subgraph):
///
/// ```text
/// LCC(v) = 2 * triangles(v) / (deg(v) * (deg(v) - 1))
/// ```
///
/// where `deg(v)` is the undirected degree and `triangles(v)` is the number
/// of triangles containing `v`.
///
/// Nodes with degree < 2 are assigned a coefficient of 0.0.
///
/// Returns a map of NodeID -> coefficient in [0.0, 1.0].
pub fn lcc(projection: &GraphProjection) -> HashMap<String, f64> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    if node_count == 0 {
        return HashMap::new();
    }

    // Get per-node triangle counts via the shared raw function
    let triangles = triangle_count_raw(projection);

    // Build undirected adjacency to compute degrees
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
    adj.par_iter_mut().for_each(|neighbors| {
        neighbors.sort_unstable();
        neighbors.dedup();
    });

    // Compute coefficients in parallel
    let coefficients: Vec<f64> = (0..node_count)
        .into_par_iter()
        .map(|u| {
            let deg = adj[u].len();
            if deg < 2 {
                0.0
            } else {
                2.0 * triangles[u] as f64 / (deg as f64 * (deg - 1) as f64)
            }
        })
        .collect();

    // Map back to String IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, &coeff) in coefficients.iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), coeff);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_lcc_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let coefficients = lcc(&projection);
        assert!(
            coefficients.is_empty(),
            "Empty graph should return empty result"
        );
    }

    #[test]
    fn test_lcc_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        assert_eq!(coefficients.len(), 1);
        assert_eq!(coefficients["A"], 0.0, "Single node should have LCC = 0.0");
    }

    #[test]
    fn test_lcc_triangle_all_ones() {
        // A-B-C with all edges -> every node has LCC = 1.0
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        assert_eq!(coefficients.len(), 3);
        assert!(
            (coefficients["A"] - 1.0).abs() < 1e-10,
            "A in a triangle should have LCC = 1.0, got {}",
            coefficients["A"]
        );
        assert!(
            (coefficients["B"] - 1.0).abs() < 1e-10,
            "B in a triangle should have LCC = 1.0, got {}",
            coefficients["B"]
        );
        assert!(
            (coefficients["C"] - 1.0).abs() < 1e-10,
            "C in a triangle should have LCC = 1.0, got {}",
            coefficients["C"]
        );
    }

    #[test]
    fn test_lcc_complete_graph_k4() {
        // K4: every pair connected -> all LCC = 1.0
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
            ("B".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        for node in &["A", "B", "C", "D"] {
            assert!(
                (coefficients[*node] - 1.0).abs() < 1e-10,
                "{} in K4 should have LCC = 1.0, got {}",
                node,
                coefficients[*node]
            );
        }
    }

    #[test]
    fn test_lcc_star_graph() {
        // Center connected to 4 spokes, no spoke-spoke edges
        // Center: deg=4, 0 triangles -> LCC = 0.0
        // Spokes: deg=1 -> LCC = 0.0
        let nodes = vec![
            "Center".to_string(),
            "S1".to_string(),
            "S2".to_string(),
            "S3".to_string(),
            "S4".to_string(),
        ];
        let edges = vec![
            ("Center".to_string(), "S1".to_string()),
            ("Center".to_string(), "S2".to_string()),
            ("Center".to_string(), "S3".to_string()),
            ("Center".to_string(), "S4".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        assert_eq!(
            coefficients["Center"], 0.0,
            "Star center has 0 triangles among neighbors"
        );
        assert_eq!(coefficients["S1"], 0.0, "Spoke with degree 1 has LCC = 0.0");
        assert_eq!(coefficients["S2"], 0.0, "Spoke with degree 1 has LCC = 0.0");
        assert_eq!(coefficients["S3"], 0.0, "Spoke with degree 1 has LCC = 0.0");
        assert_eq!(coefficients["S4"], 0.0, "Spoke with degree 1 has LCC = 0.0");
    }

    #[test]
    fn test_lcc_path_graph() {
        // A - B - C (path): A deg=1, B deg=2 (0 triangles), C deg=1
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        assert_eq!(coefficients["A"], 0.0, "A has degree 1, LCC = 0.0");
        assert_eq!(
            coefficients["B"], 0.0,
            "B has degree 2 but no triangle, LCC = 0.0"
        );
        assert_eq!(coefficients["C"], 0.0, "C has degree 1, LCC = 0.0");
    }

    #[test]
    fn test_lcc_two_triangles_sharing_edge() {
        // Two triangles sharing edge B-C:
        // Triangle 1: A-B-C
        // Triangle 2: B-C-D
        //
        // A: deg=2, 1 triangle -> LCC = 2*1/(2*1) = 1.0
        // B: deg=3, 2 triangles -> LCC = 2*2/(3*2) = 4/6 = 0.666...
        // C: deg=3, 2 triangles -> LCC = 2*2/(3*2) = 4/6 = 0.666...
        // D: deg=2, 1 triangle -> LCC = 2*1/(2*1) = 1.0
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
            ("C".to_string(), "D".to_string()),
            ("D".to_string(), "B".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let coefficients = lcc(&projection);

        assert!(
            (coefficients["A"] - 1.0).abs() < 1e-10,
            "A: deg=2, 1 triangle -> LCC = 1.0, got {}",
            coefficients["A"]
        );
        let expected_bc = 2.0 / 3.0;
        assert!(
            (coefficients["B"] - expected_bc).abs() < 1e-10,
            "B: deg=3, 2 triangles -> LCC = 2/3, got {}",
            coefficients["B"]
        );
        assert!(
            (coefficients["C"] - expected_bc).abs() < 1e-10,
            "C: deg=3, 2 triangles -> LCC = 2/3, got {}",
            coefficients["C"]
        );
        assert!(
            (coefficients["D"] - 1.0).abs() < 1e-10,
            "D: deg=2, 1 triangle -> LCC = 1.0, got {}",
            coefficients["D"]
        );
    }
}
