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

//! Betweenness Centrality via Brandes' algorithm on CSR.
//!
//! Betweenness centrality measures how often a node lies on the shortest path
//! between two other nodes. Nodes with high betweenness are "bridges" that
//! connect different parts of the graph.
//!
//! Time Complexity: O(V * E) for unweighted graphs
//! Space Complexity: O(V + E)

use crate::projection::GraphProjection;
use rayon::prelude::*;
use std::collections::{HashMap, VecDeque};

/// Compute betweenness centrality for all nodes using Brandes' algorithm.
///
/// For each source node, BFS computes shortest path counts and distances,
/// then backtracking accumulates dependency scores. The per-source passes
/// are parallelized with rayon since each BFS is independent.
///
/// Results are normalized by `(n-1)(n-2)` for directed graphs.
///
/// Returns a map of NodeID (string) -> betweenness score.
pub fn betweenness_centrality(projection: &GraphProjection) -> HashMap<String, f64> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    if node_count <= 2 {
        // Betweenness is only meaningful for graphs with 3+ nodes
        let mut result = HashMap::with_capacity(node_count);
        for i in 0..node_count {
            if let Some(node_id) = projection.get_node_id(i as u32) {
                result.insert(node_id.clone(), 0.0);
            }
        }
        return result;
    }

    // Run Brandes' BFS from each source node in parallel.
    // Each source produces a Vec<f64> of per-node betweenness contributions.
    let partial_scores: Vec<Vec<f64>> = (0..node_count)
        .into_par_iter()
        .map(|s| {
            let mut stack: Vec<u32> = Vec::new();
            let mut predecessors: Vec<Vec<u32>> = vec![Vec::new(); node_count];
            let mut sigma: Vec<f64> = vec![0.0; node_count]; // shortest path counts
            let mut dist: Vec<i64> = vec![-1; node_count];

            sigma[s] = 1.0;
            dist[s] = 0;

            let mut queue = VecDeque::new();
            queue.push_back(s as u32);

            // BFS phase
            while let Some(v) = queue.pop_front() {
                stack.push(v);
                let v_dist = dist[v as usize];

                if (v as usize) < graph.node_count() {
                    for &w in graph.neighbors_slice(v) {
                        let w_usize = w as usize;
                        // First visit
                        if dist[w_usize] < 0 {
                            queue.push_back(w);
                            dist[w_usize] = v_dist + 1;
                        }
                        // Shortest path via v?
                        if dist[w_usize] == v_dist + 1 {
                            sigma[w_usize] += sigma[v as usize];
                            predecessors[w_usize].push(v);
                        }
                    }
                }
            }

            // Accumulation phase (backtrack)
            let mut delta: Vec<f64> = vec![0.0; node_count];
            let mut contribution: Vec<f64> = vec![0.0; node_count];
            while let Some(w) = stack.pop() {
                let w_usize = w as usize;
                for &v in &predecessors[w_usize] {
                    let v_usize = v as usize;
                    let coeff = (sigma[v_usize] / sigma[w_usize]) * (1.0 + delta[w_usize]);
                    delta[v_usize] += coeff;
                }
                // Don't add source's own contribution
                if w_usize != s {
                    contribution[w_usize] = delta[w_usize];
                }
            }

            contribution
        })
        .collect();

    // Sum partial scores across all source nodes
    let mut totals = vec![0.0_f64; node_count];
    for partial in &partial_scores {
        for (i, &score) in partial.iter().enumerate() {
            totals[i] += score;
        }
    }

    // Normalize by (n-1)(n-2) for directed graphs
    let normalization = ((node_count - 1) * (node_count - 2)) as f64;
    if normalization > 0.0 {
        for score in totals.iter_mut() {
            *score /= normalization;
        }
    }

    // Map back to string IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, score) in totals.into_iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), score);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_betweenness_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let scores = betweenness_centrality(&projection);
        assert!(scores.is_empty(), "Empty graph should return empty result");
    }

    #[test]
    fn test_betweenness_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = betweenness_centrality(&projection);

        assert_eq!(scores.len(), 1);
        assert_eq!(scores["A"], 0.0, "Single node should have betweenness 0.0");
    }

    #[test]
    fn test_betweenness_line_graph() {
        // Directed line: A -> B -> C -> D
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

        let scores = betweenness_centrality(&projection);

        assert_eq!(scores.len(), 4);
        // Middle nodes B and C should have higher betweenness than endpoints
        assert!(
            scores["B"] > scores["A"],
            "B ({}) should have higher betweenness than A ({})",
            scores["B"],
            scores["A"]
        );
        assert!(
            scores["C"] > scores["D"],
            "C ({}) should have higher betweenness than D ({})",
            scores["C"],
            scores["D"]
        );
        // Endpoints should have 0 betweenness (no paths pass through them)
        assert_eq!(scores["A"], 0.0, "Endpoint A should have betweenness 0.0");
        assert_eq!(scores["D"], 0.0, "Endpoint D should have betweenness 0.0");
    }

    #[test]
    fn test_betweenness_triangle() {
        // Directed cycle: A -> B -> C -> A
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = betweenness_centrality(&projection);

        assert_eq!(scores.len(), 3);
        // In a directed cycle, all nodes have equal betweenness
        let a_score = scores["A"];
        assert!(
            (scores["B"] - a_score).abs() < 1e-10,
            "All nodes in a directed cycle should have equal betweenness"
        );
        assert!(
            (scores["C"] - a_score).abs() < 1e-10,
            "All nodes in a directed cycle should have equal betweenness"
        );
        // All scores >= 0
        for (_, &score) in &scores {
            assert!(score >= 0.0, "Betweenness should be non-negative");
        }
    }

    #[test]
    fn test_betweenness_star_graph() {
        // Bidirectional star: Center <-> S1, Center <-> S2, Center <-> S3
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
            ("S1".to_string(), "Center".to_string()),
            ("S2".to_string(), "Center".to_string()),
            ("S3".to_string(), "Center".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = betweenness_centrality(&projection);

        assert_eq!(scores.len(), 4);
        // Center should have highest betweenness (all paths go through it)
        assert!(
            scores["Center"] > scores["S1"],
            "Center ({}) should have higher betweenness than S1 ({})",
            scores["Center"],
            scores["S1"]
        );
        // Spokes should have 0 betweenness
        assert_eq!(scores["S1"], 0.0, "Spoke S1 should have betweenness 0.0");
        assert_eq!(scores["S2"], 0.0, "Spoke S2 should have betweenness 0.0");
        assert_eq!(scores["S3"], 0.0, "Spoke S3 should have betweenness 0.0");
    }
}
