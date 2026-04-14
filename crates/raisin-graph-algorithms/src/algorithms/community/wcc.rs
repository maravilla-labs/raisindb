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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Weakly Connected Components (parallel min-label propagation)
///
/// Finds connected components in the graph by ignoring edge direction.
/// Each node is assigned a component label equal to the smallest node index
/// reachable from it via undirected edges. Convergence is achieved when no
/// label changes in an iteration.
///
/// Returns a map of NodeID -> ComponentID.
pub fn weakly_connected_components(projection: &GraphProjection) -> HashMap<String, u32> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    if node_count == 0 {
        return HashMap::new();
    }

    // Build undirected adjacency from forward CSR edges
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); node_count];
    for u in 0..node_count {
        if u < graph.node_count() {
            for &v in graph.neighbors_slice(u as u32) {
                adj[u].push(v);
                adj[v as usize].push(u as u32);
            }
        }
    }
    adj.par_iter_mut().for_each(|neighbors| {
        neighbors.sort_unstable();
        neighbors.dedup();
    });

    // Initialize labels: labels[i] = i
    let labels: Vec<AtomicU32> = (0..node_count as u32).map(AtomicU32::new).collect();

    // Iterate until convergence
    loop {
        let changed = AtomicBool::new(false);

        (0..node_count).into_par_iter().for_each(|u| {
            let mut min_label = labels[u].load(Ordering::Relaxed);
            for &v in &adj[u] {
                let v_label = labels[v as usize].load(Ordering::Relaxed);
                if v_label < min_label {
                    min_label = v_label;
                }
            }
            if min_label < labels[u].load(Ordering::Relaxed) {
                labels[u].fetch_min(min_label, Ordering::Relaxed);
                changed.store(true, Ordering::Release);
            }
        });

        if !changed.load(Ordering::Acquire) {
            break;
        }
    }

    // Map back to String IDs
    let mut result = HashMap::with_capacity(node_count);
    for (i, label) in labels.iter().enumerate() {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), label.load(Ordering::Relaxed));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::GraphProjection;

    #[test]
    fn test_wcc() {
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())]; // C is isolated
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);
        assert_eq!(components["A"], components["B"]);
        assert_ne!(components["A"], components["C"]);
    }

    #[test]
    fn test_wcc_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let components = weakly_connected_components(&projection);
        assert!(
            components.is_empty(),
            "Empty graph should return empty components"
        );
    }

    #[test]
    fn test_wcc_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);

        assert_eq!(components.len(), 1);
        assert!(components.contains_key("A"));
    }

    #[test]
    fn test_wcc_transitivity() {
        // A-B, B-C => A, B, C should be in same component
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);

        assert_eq!(
            components["A"], components["B"],
            "A and B should be in same component"
        );
        assert_eq!(
            components["B"], components["C"],
            "B and C should be in same component"
        );
        assert_eq!(
            components["A"], components["C"],
            "A and C should be in same component (transitivity)"
        );
    }

    #[test]
    fn test_wcc_multiple_components() {
        // Three separate components: {A,B}, {C,D}, {E}
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
        ];
        let edges = vec![
            ("A".to_string(), "B".to_string()),
            ("C".to_string(), "D".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);

        // Check groupings
        assert_eq!(
            components["A"], components["B"],
            "A and B should be in same component"
        );
        assert_eq!(
            components["C"], components["D"],
            "C and D should be in same component"
        );

        // Check separations
        assert_ne!(
            components["A"], components["C"],
            "Component {{A,B}} should differ from {{C,D}}"
        );
        assert_ne!(
            components["A"], components["E"],
            "Component {{A,B}} should differ from {{E}}"
        );
        assert_ne!(
            components["C"], components["E"],
            "Component {{C,D}} should differ from {{E}}"
        );

        // Count unique components
        let unique: std::collections::HashSet<_> = components.values().collect();
        assert_eq!(unique.len(), 3, "Should have exactly 3 components");
    }

    #[test]
    fn test_wcc_directed_treated_as_undirected() {
        // A -> B (directed), but WCC should treat as undirected
        let nodes = vec!["A".to_string(), "B".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())];
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);

        assert_eq!(
            components["A"], components["B"],
            "Directed edge A->B should connect A and B in WCC"
        );
    }
}
