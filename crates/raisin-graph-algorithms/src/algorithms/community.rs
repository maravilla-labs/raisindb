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
use std::collections::HashMap;

/// Weakly Connected Components
///
/// Finds connected components in the graph (ignoring edge direction).
/// Returns a map of NodeID -> ComponentID.
pub fn weakly_connected_components(projection: &GraphProjection) -> HashMap<String, u32> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    // petgraph has a built-in algo for this, but let's implement a simple BFS/Union-Find
    // Since Csr is directed, we need to treat it as undirected.
    // Actually, petgraph::algo::connected_components works on Csr.

    // let components = petgraph::algo::connected_components(graph);

    // The return is just the number of components.
    // To get the component ID for each node, we need `tarjan_scc` or similar,
    // but that's for Strongly Connected.

    // Let's implement a simple Union-Find for WCC
    let mut parent: Vec<usize> = (0..node_count).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] == i {
            i
        } else {
            let root = find(parent, parent[i]);
            parent[i] = root; // Path compression
            root
        }
    }

    fn union(parent: &mut [usize], i: usize, j: usize) {
        let root_i = find(parent, i);
        let root_j = find(parent, j);
        if root_i != root_j {
            parent[root_i] = root_j;
        }
    }

    // Iterate all edges
    // Csr stores edges as adjacency lists
    for u in 0..node_count {
        if u < graph.node_count() {
            for &v in graph.neighbors_slice(u as u32) {
                union(&mut parent, u, v as usize);
            }
        }
    }

    // Build result map
    let mut result = HashMap::with_capacity(node_count);
    for i in 0..node_count {
        let root = find(&mut parent, i);
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), root as u32);
        }
    }

    result
}

/// Louvain Community Detection
///
/// Detects communities by optimizing modularity.
/// Returns a map of NodeID -> CommunityID.
pub fn louvain(
    projection: &GraphProjection,
    iterations: usize,
    resolution: f64,
) -> HashMap<String, u32> {
    let graph = projection.graph();
    let node_count = projection.node_count();

    if node_count == 0 {
        return HashMap::new();
    }

    // 1. Initialize each node in its own community
    let mut community: Vec<usize> = (0..node_count).collect();

    // Calculate total graph weight (m) and node degrees (k_i)
    // Assuming unweighted graph for now, so weight = 1.0 for each edge
    let mut k = vec![0.0; node_count];
    let mut m = 0.0;

    for (u, k_u) in k.iter_mut().enumerate().take(node_count) {
        let degree = if u < graph.node_count() {
            graph.neighbors_slice(u as u32).len() as f64
        } else {
            0.0
        };
        *k_u = degree;
        m += degree;
    }

    if m == 0.0 {
        m = 1.0; // Avoid division by zero
    }

    // Track community weights
    // tot[c] = sum of degrees of nodes in community c
    let mut tot = k.clone();

    let mut improved = true;
    let mut iter = 0;

    while improved && iter < iterations {
        improved = false;
        iter += 1;

        for u in 0..node_count {
            let current_comm = community[u];
            let k_u = k[u];

            // Remove u from its community
            tot[current_comm] -= k_u;

            // Find best neighbor community
            // k_u_c[c] = sum of weights from u to community c
            let mut k_u_c: HashMap<usize, f64> = HashMap::new();

            // Check neighbors
            if u < graph.node_count() {
                for &v in graph.neighbors_slice(u as u32) {
                    let v_comm = community[v as usize];
                    *k_u_c.entry(v_comm).or_default() += 1.0;
                }
            }

            let mut best_comm = current_comm;
            let mut max_gain = 0.0;

            // Check all neighbor communities and current (if not in neighbors)
            let mut candidates: Vec<usize> = k_u_c.keys().cloned().collect();
            if !k_u_c.contains_key(&current_comm) {
                candidates.push(current_comm);
            }

            for target_comm in candidates {
                let k_c_in = *k_u_c.get(&target_comm).unwrap_or(&0.0);
                let tot_c = tot[target_comm];

                // Modularity gain formula
                let gain = k_c_in - (k_u * tot_c * resolution) / m;

                if gain > max_gain {
                    max_gain = gain;
                    best_comm = target_comm;
                }
            }

            // Apply move
            community[u] = best_comm;
            tot[best_comm] += k_u;

            if best_comm != current_comm {
                improved = true;
            }
        }
    }

    // Map results
    let mut result = HashMap::with_capacity(node_count);
    for (i, &comm) in community.iter().enumerate().take(node_count) {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            result.insert(node_id.clone(), comm as u32);
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
    fn test_wcc() {
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges = vec![("A".to_string(), "B".to_string())]; // C is isolated
        let projection = GraphProjection::from_parts(nodes, edges);

        let components = weakly_connected_components(&projection);
        assert_eq!(components["A"], components["B"]);
        assert_ne!(components["A"], components["C"]);
    }

    // ==================== Louvain Tests ====================

    #[test]
    fn test_louvain_empty_graph() {
        let projection = GraphProjection::from_parts(vec![], vec![]);
        let communities = louvain(&projection, 10, 1.0);
        assert!(
            communities.is_empty(),
            "Empty graph should return empty communities"
        );
    }

    #[test]
    fn test_louvain_single_node() {
        let nodes = vec!["A".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let communities = louvain(&projection, 10, 1.0);

        assert_eq!(communities.len(), 1);
        assert!(
            communities.contains_key("A"),
            "Single node should be assigned a community"
        );
    }

    #[test]
    fn test_louvain_all_nodes_assigned() {
        let projection = create_test_graph();
        let communities = louvain(&projection, 10, 1.0);

        // All 5 nodes should be assigned
        assert_eq!(communities.len(), 5);
        assert!(communities.contains_key("A"));
        assert!(communities.contains_key("B"));
        assert!(communities.contains_key("C"));
        assert!(communities.contains_key("D"));
        assert!(communities.contains_key("E"));
    }

    #[test]
    fn test_louvain_no_edges_separate_communities() {
        // Nodes with no edges should (potentially) be in their own communities
        let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let edges: Vec<(String, String)> = vec![];
        let projection = GraphProjection::from_parts(nodes, edges);

        let communities = louvain(&projection, 10, 1.0);

        // All nodes should be assigned
        assert_eq!(communities.len(), 3);
    }

    #[test]
    fn test_louvain_clique_same_community() {
        // Complete graph of 4 nodes (clique) - should be in same community
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
            ("B".to_string(), "A".to_string()),
            ("B".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
            ("C".to_string(), "A".to_string()),
            ("C".to_string(), "B".to_string()),
            ("C".to_string(), "D".to_string()),
            ("D".to_string(), "A".to_string()),
            ("D".to_string(), "B".to_string()),
            ("D".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let communities = louvain(&projection, 20, 1.0);

        // In a clique, all nodes should typically end up in the same community
        let comm_a = communities["A"];
        assert_eq!(
            communities["B"], comm_a,
            "Clique members should be in same community"
        );
        assert_eq!(
            communities["C"], comm_a,
            "Clique members should be in same community"
        );
        assert_eq!(
            communities["D"], comm_a,
            "Clique members should be in same community"
        );
    }

    #[test]
    fn test_louvain_disconnected_different_communities() {
        // Two disconnected cliques
        // Clique 1: A-B-C
        // Clique 2: X-Y-Z
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "X".to_string(),
            "Y".to_string(),
            "Z".to_string(),
        ];
        let edges = vec![
            // Clique 1
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "A".to_string()),
            ("A".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "B".to_string()),
            // Clique 2
            ("X".to_string(), "Y".to_string()),
            ("Y".to_string(), "X".to_string()),
            ("X".to_string(), "Z".to_string()),
            ("Z".to_string(), "X".to_string()),
            ("Y".to_string(), "Z".to_string()),
            ("Z".to_string(), "Y".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let communities = louvain(&projection, 20, 1.0);

        // Nodes in same clique should be in same community
        assert_eq!(communities["A"], communities["B"]);
        assert_eq!(communities["A"], communities["C"]);
        assert_eq!(communities["X"], communities["Y"]);
        assert_eq!(communities["X"], communities["Z"]);

        // Different cliques should be in different communities
        assert_ne!(
            communities["A"], communities["X"],
            "Disconnected cliques should be in different communities"
        );
    }

    #[test]
    fn test_louvain_resolution_parameter() {
        // Higher resolution should produce more communities
        // Two weakly connected groups
        let nodes = vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
            "F".to_string(),
        ];
        // Group 1: A-B-C (dense)
        // Group 2: D-E-F (dense)
        // Single weak link: C-D
        let edges = vec![
            // Group 1
            ("A".to_string(), "B".to_string()),
            ("B".to_string(), "A".to_string()),
            ("A".to_string(), "C".to_string()),
            ("C".to_string(), "A".to_string()),
            ("B".to_string(), "C".to_string()),
            ("C".to_string(), "B".to_string()),
            // Group 2
            ("D".to_string(), "E".to_string()),
            ("E".to_string(), "D".to_string()),
            ("D".to_string(), "F".to_string()),
            ("F".to_string(), "D".to_string()),
            ("E".to_string(), "F".to_string()),
            ("F".to_string(), "E".to_string()),
            // Weak link
            ("C".to_string(), "D".to_string()),
            ("D".to_string(), "C".to_string()),
        ];
        let projection = GraphProjection::from_parts(nodes, edges);

        let low_res = louvain(&projection, 20, 0.5);
        let high_res = louvain(&projection, 20, 2.0);

        // With higher resolution, groups should be more separated
        let low_res_unique: std::collections::HashSet<_> = low_res.values().collect();
        let high_res_unique: std::collections::HashSet<_> = high_res.values().collect();

        // Higher resolution should produce at least as many communities
        assert!(
            high_res_unique.len() >= low_res_unique.len(),
            "Higher resolution should produce more or equal communities: low={}, high={}",
            low_res_unique.len(),
            high_res_unique.len()
        );
    }

    // ==================== WCC Additional Tests ====================

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
