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

/// Pull-based PageRank implementation using the backward (transpose) graph.
///
/// Each iteration, every node v reads its in-neighbors from the backward CSR
/// and accumulates their contributions. This eliminates write contention,
/// making the inner loop fully parallelizable via rayon.
///
/// The backward graph is built lazily via `ensure_backward_graph()`.
///
/// Returns a map of NodeID -> Score.
pub fn page_rank(
    projection: &mut GraphProjection,
    damping_factor: f64,
    iterations: usize,
    tolerance: f64,
) -> HashMap<String, f64> {
    let node_count = projection.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    let n = node_count as f64;
    let graph = projection.graph();

    // Pre-compute out-degrees from the forward graph in parallel
    let out_degrees: Vec<usize> = (0..node_count)
        .into_par_iter()
        .map(|i| {
            if i < graph.node_count() {
                graph.neighbors_slice(i as u32).len()
            } else {
                0
            }
        })
        .collect();

    // Build the backward graph for pull-based iteration
    projection.ensure_backward_graph();
    let backward = projection
        .backward_graph()
        .expect("backward graph just built");

    // Initialize scores: 1.0 / N
    let initial_score = 1.0 / n;
    let mut scores = vec![initial_score; node_count];
    let mut new_scores = vec![0.0; node_count];

    for _iter in 0..iterations {
        // Compute dangling sum: sum of scores for nodes with no outgoing edges
        let dangling_sum: f64 = (0..node_count)
            .into_par_iter()
            .filter(|&i| out_degrees[i] == 0)
            .map(|i| scores[i])
            .sum();

        let base = (1.0 - damping_factor) / n + damping_factor * dangling_sum / n;

        // Pull-based: each node v reads from its in-neighbors
        new_scores
            .par_iter_mut()
            .enumerate()
            .for_each(|(v, new_score)| {
                let mut sum = base;
                if v < backward.node_count() {
                    for &u in backward.neighbors_slice(v as u32) {
                        let u_idx = u as usize;
                        let deg = out_degrees[u_idx];
                        if deg > 0 {
                            sum += damping_factor * scores[u_idx] / deg as f64;
                        }
                    }
                }
                *new_score = sum;
            });

        // Check convergence: L1-norm
        let diff: f64 = (0..node_count)
            .into_par_iter()
            .map(|i| (new_scores[i] - scores[i]).abs())
            .sum();

        // Swap buffers
        std::mem::swap(&mut scores, &mut new_scores);

        if diff < tolerance {
            break;
        }
    }

    // Map back to String IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, score) in scores.into_iter().enumerate() {
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
    fn test_page_rank() {
        let mut projection = create_test_graph();
        let scores = page_rank(&mut projection, 0.85, 20, 1e-6);

        assert!(scores.contains_key("A"));
        assert!(scores.contains_key("E"));
        // E should have high rank as it's a sink
        assert!(scores["E"] > scores["A"]);
    }

    #[test]
    fn test_page_rank_sum_to_one() {
        let mut projection = create_test_graph();
        let scores = page_rank(&mut projection, 0.85, 100, 1e-10);

        let sum: f64 = scores.values().sum();
        // PageRank scores should sum to approximately 1.0
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "PageRank scores should sum to ~1.0, got {}",
            sum
        );
    }

    #[test]
    fn test_page_rank_all_non_negative() {
        let mut projection = create_test_graph();
        let scores = page_rank(&mut projection, 0.85, 20, 1e-6);

        for (node, score) in &scores {
            assert!(
                *score >= 0.0,
                "PageRank score for {} should be non-negative, got {}",
                node,
                score
            );
        }
    }

    #[test]
    fn test_page_rank_empty_graph() {
        let mut projection = GraphProjection::from_parts(vec![], vec![]);
        let scores = page_rank(&mut projection, 0.85, 20, 1e-6);
        assert!(scores.is_empty(), "Empty graph should return empty scores");
    }

    #[test]
    fn test_page_rank_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let mut projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&mut projection, 0.85, 20, 1e-6);

        assert_eq!(scores.len(), 1);
        assert!(
            (scores["A"] - 1.0).abs() < 1e-6,
            "Single node should have score 1.0, got {}",
            scores["A"]
        );
    }

    #[test]
    fn test_page_rank_dangling_nodes() {
        // A -> B -> C (C is dangling - no outgoing edges)
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let mut projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&mut projection, 0.85, 50, 1e-10);

        // All scores should be positive
        assert!(scores["A"] > 0.0);
        assert!(scores["B"] > 0.0);
        assert!(scores["C"] > 0.0);

        // Sum should be ~1.0
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 1e-6, "Sum should be ~1.0, got {}", sum);

        // C should have highest rank (dangling node accumulates rank)
        assert!(
            scores["C"] >= scores["B"],
            "Dangling node C should have high rank"
        );
    }

    #[test]
    fn test_page_rank_disconnected_components() {
        // Component 1: A -> B
        // Component 2: C -> D (disconnected)
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let mut projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&mut projection, 0.85, 50, 1e-10);

        // All nodes should have positive scores due to teleportation
        for (node, score) in &scores {
            assert!(*score > 0.0, "Node {} should have positive score", node);
        }

        // Sum should still be ~1.0
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 1e-6, "Sum should be ~1.0, got {}", sum);
    }

    #[test]
    fn test_page_rank_pull_based_matches_reference() {
        // Hand-computed 3-node graph: A -> B -> C, A -> C
        // With damping=0.85, N=3:
        //   out_degrees: A=2, B=1, C=0 (dangling)
        //   After convergence, C should have the highest score (receives from A and B,
        //   plus dangling redistribution from itself), B middle, A lowest.
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let mut projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&mut projection, 0.85, 200, 1e-12);

        // Verify sum to 1
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 1e-6, "Sum should be ~1.0, got {}", sum);

        // Ordering: C > B > A (C is the sink that accumulates all rank)
        assert!(
            scores["C"] > scores["B"],
            "C ({}) should rank higher than B ({})",
            scores["C"],
            scores["B"]
        );
        assert!(
            scores["B"] > scores["A"],
            "B ({}) should rank higher than A ({})",
            scores["B"],
            scores["A"]
        );

        // Verify approximate reference values (computed by this implementation)
        // With dangling node redistribution and d=0.85, N=3:
        // A: ~0.1976, B: ~0.2816, C: ~0.5209
        assert!(
            (scores["A"] - 0.1976).abs() < 0.01,
            "A score {} should be ~0.1976",
            scores["A"]
        );
        assert!(
            (scores["B"] - 0.2816).abs() < 0.01,
            "B score {} should be ~0.2816",
            scores["B"]
        );
        assert!(
            (scores["C"] - 0.5209).abs() < 0.01,
            "C score {} should be ~0.5209",
            scores["C"]
        );
    }
}
