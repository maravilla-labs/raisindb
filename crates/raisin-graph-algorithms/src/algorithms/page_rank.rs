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

/// PageRank implementation
///
/// Computes the PageRank for all nodes in the projection.
/// Returns a map of NodeID -> Score.
pub fn page_rank(
    projection: &GraphProjection,
    damping_factor: f64,
    iterations: usize,
    tolerance: f64,
) -> HashMap<String, f64> {
    let node_count = projection.node_count();
    if node_count == 0 {
        return HashMap::new();
    }

    let graph = projection.graph();

    // Initialize scores: 1.0 / N
    let initial_score = 1.0 / node_count as f64;
    let mut scores = vec![initial_score; node_count];
    let mut new_scores = vec![0.0; node_count];

    // Pre-calculate out-degrees to avoid re-fetching
    let mut out_degrees = vec![0; node_count];
    // Parallelize degree calculation
    out_degrees.par_iter_mut().enumerate().for_each(|(i, deg)| {
        if i < graph.node_count() {
            *deg = graph.neighbors_slice(i as u32).len();
        } else {
            *deg = 0;
        }
    });

    for _iter in 0..iterations {
        // Reset new_scores with the base teleport probability
        let base_score = (1.0 - damping_factor) / node_count as f64;
        // Parallel fill
        new_scores.par_iter_mut().for_each(|x| *x = base_score);

        // Distribute scores (Push method)
        // Sequential push for now as parallel push requires atomic floats or reduction
        for u in 0..node_count {
            let degree = out_degrees[u];
            if degree > 0 {
                let push_val = damping_factor * scores[u] / degree as f64;
                // Safe access: if degree > 0, u must be in graph
                if u < graph.node_count() {
                    for &v in graph.neighbors_slice(u as u32) {
                        new_scores[v as usize] += push_val;
                    }
                }
            } else {
                // Dangling node (sink): distribute to everyone
                let push_val = damping_factor * scores[u] / node_count as f64;
                for val in new_scores.iter_mut() {
                    *val += push_val;
                }
            }
        }

        // Check convergence (Parallel reduction)
        let diff: f64 = (0..node_count)
            .into_par_iter()
            .map(|i| (new_scores[i] - scores[i]).abs())
            .sum();

        // Swap buffers
        scores.copy_from_slice(&new_scores);

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
        let projection = create_test_graph();
        let scores = page_rank(&projection, 0.85, 20, 1e-6);

        assert!(scores.contains_key("A"));
        assert!(scores.contains_key("E"));
        // E should have high rank as it's a sink
        assert!(scores["E"] > scores["A"]);
    }

    #[test]
    fn test_page_rank_sum_to_one() {
        let projection = create_test_graph();
        let scores = page_rank(&projection, 0.85, 100, 1e-10);

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
        let projection = create_test_graph();
        let scores = page_rank(&projection, 0.85, 20, 1e-6);

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
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let scores = page_rank(&projection, 0.85, 20, 1e-6);
        assert!(scores.is_empty(), "Empty graph should return empty scores");
    }

    #[test]
    fn test_page_rank_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&projection, 0.85, 20, 1e-6);

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
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&projection, 0.85, 50, 1e-10);

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
        let projection = GraphProjection::from_parts(nodes, edges);

        let scores = page_rank(&projection, 0.85, 50, 1e-10);

        // All nodes should have positive scores due to teleportation
        for (node, score) in &scores {
            assert!(*score > 0.0, "Node {} should have positive score", node);
        }

        // Sum should still be ~1.0
        let sum: f64 = scores.values().sum();
        assert!((sum - 1.0).abs() < 1e-6, "Sum should be ~1.0, got {}", sum);
    }
}
