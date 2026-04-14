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

/// Internal weighted graph representation for Louvain community aggregation.
///
/// Supports self-loops and weighted edges, which are needed when contracting
/// communities into super-nodes during Phase 2.
struct InternalGraph {
    /// Number of nodes (may be contracted super-nodes)
    node_count: usize,
    /// Adjacency list with weights: node -> [(neighbor, weight)]
    adj: Vec<Vec<(usize, f64)>>,
    /// Self-loop weights per node
    self_loops: Vec<f64>,
    /// Total degree (sum of edge weights) per node
    degrees: Vec<f64>,
    /// Total edge weight in graph (sum of all edge weights, counting each undirected edge once)
    total_weight: f64,
}

impl InternalGraph {
    /// Build an InternalGraph from a GraphProjection's CSR, treating directed edges
    /// as undirected by summing both directions.
    fn from_projection(projection: &GraphProjection) -> Self {
        let graph = projection.graph();
        let node_count = projection.node_count();

        // Build undirected adjacency with weights
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); node_count];
        let mut total_weight = 0.0;

        for u in 0..node_count {
            if u < graph.node_count() {
                for &v in graph.neighbors_slice(u as u32) {
                    let v = v as usize;
                    if u == v {
                        // Self-loop: handled separately below
                        continue;
                    }
                    *adj[u].entry(v).or_default() += 1.0;
                    *adj[v].entry(u).or_default() += 1.0;
                }
            }
        }

        // Convert to Vec<Vec<(usize, f64)>>
        let adj_vec: Vec<Vec<(usize, f64)>> = adj
            .into_iter()
            .map(|map| map.into_iter().collect())
            .collect();

        // Compute degrees and total weight
        let mut degrees = vec![0.0; node_count];
        for (u, neighbors) in adj_vec.iter().enumerate() {
            for &(_, w) in neighbors {
                degrees[u] += w;
            }
        }

        for d in &degrees {
            total_weight += d;
        }

        InternalGraph {
            node_count,
            adj: adj_vec,
            self_loops: vec![0.0; node_count],
            degrees,
            total_weight,
        }
    }

    /// Run Phase 1: local greedy modularity moves.
    /// Returns the community assignment and whether any improvement was made.
    fn phase1(&self, max_iterations: usize, resolution: f64) -> (Vec<usize>, bool) {
        let n = self.node_count;
        let mut community: Vec<usize> = (0..n).collect();
        let m = if self.total_weight == 0.0 {
            1.0
        } else {
            self.total_weight
        };

        // tot[c] = sum of degrees of nodes in community c
        let mut tot: Vec<f64> = self.degrees.clone();
        // in_weight[c] = 2 * sum of internal edge weights in community c
        let mut sigma_in: Vec<f64> = self.self_loops.iter().map(|sl| 2.0 * sl).collect();

        let mut any_moved = false;
        let mut improved = true;
        let mut iter = 0;

        while improved && iter < max_iterations {
            improved = false;
            iter += 1;

            for u in 0..n {
                let current_comm = community[u];
                let k_u = self.degrees[u];

                if k_u == 0.0 && self.self_loops[u] == 0.0 {
                    continue; // Isolated node, skip
                }

                // Compute weights from u to each neighboring community
                let mut k_u_c: HashMap<usize, f64> = HashMap::new();
                for &(v, w) in &self.adj[u] {
                    let v_comm = community[v];
                    *k_u_c.entry(v_comm).or_default() += w;
                }

                // Remove u from its current community
                tot[current_comm] -= k_u;
                sigma_in[current_comm] -=
                    2.0 * k_u_c.get(&current_comm).unwrap_or(&0.0) + 2.0 * self.self_loops[u];

                let mut best_comm = current_comm;
                let mut max_gain = 0.0;

                // Candidates: all neighbor communities + current
                let mut candidates: Vec<usize> = k_u_c.keys().cloned().collect();
                if !k_u_c.contains_key(&current_comm) {
                    candidates.push(current_comm);
                }

                for target_comm in candidates {
                    let k_c_in = *k_u_c.get(&target_comm).unwrap_or(&0.0);
                    let tot_c = tot[target_comm];

                    // Modularity gain = k_c_in - resolution * k_u * tot_c / m
                    let gain = k_c_in - (k_u * tot_c * resolution) / m;

                    if gain > max_gain {
                        max_gain = gain;
                        best_comm = target_comm;
                    }
                }

                // Apply move
                community[u] = best_comm;
                tot[best_comm] += k_u;
                sigma_in[best_comm] +=
                    2.0 * k_u_c.get(&best_comm).unwrap_or(&0.0) + 2.0 * self.self_loops[u];

                if best_comm != current_comm {
                    improved = true;
                    any_moved = true;
                }
            }
        }

        (community, any_moved)
    }

    /// Phase 2: Contract the graph by merging nodes in the same community
    /// into super-nodes. Returns the contracted graph and the mapping from
    /// old node index to new super-node index.
    fn contract(&self, community: &[usize]) -> (InternalGraph, Vec<usize>) {
        // Renumber communities to be contiguous 0..num_communities
        let mut comm_to_new: HashMap<usize, usize> = HashMap::new();
        let mut next_id = 0usize;
        let mapping: Vec<usize> = community
            .iter()
            .map(|&c| {
                let len = comm_to_new.len();
                *comm_to_new.entry(c).or_insert_with(|| {
                    let id = len;
                    next_id = len + 1;
                    id
                })
            })
            .collect();
        let _ = next_id; // suppress warning
        let num_communities = comm_to_new.len();

        // Build new adjacency
        let mut new_adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); num_communities];
        let mut new_self_loops = vec![0.0; num_communities];

        for u in 0..self.node_count {
            let cu = mapping[u];
            new_self_loops[cu] += self.self_loops[u];

            for &(v, w) in &self.adj[u] {
                let cv = mapping[v];
                if cu == cv {
                    new_self_loops[cu] += w / 2.0;
                } else {
                    *new_adj[cu].entry(cv).or_default() += w;
                }
            }
        }

        // Halve inter-community weights (symmetric double-counting)
        for cu in 0..num_communities {
            for (_, w) in new_adj[cu].iter_mut() {
                *w /= 2.0;
            }
        }

        let adj_vec: Vec<Vec<(usize, f64)>> = new_adj
            .into_iter()
            .map(|map| map.into_iter().collect())
            .collect();

        let mut degrees = vec![0.0; num_communities];
        let mut total_weight = 0.0;
        for (u, neighbors) in adj_vec.iter().enumerate() {
            for &(_, w) in neighbors {
                degrees[u] += w;
            }
            // Self-loops contribute to degree too (each self-loop adds 2 to degree)
            degrees[u] += 2.0 * new_self_loops[u];
        }
        for d in &degrees {
            total_weight += d;
        }

        let contracted = InternalGraph {
            node_count: num_communities,
            adj: adj_vec,
            self_loops: new_self_loops,
            degrees,
            total_weight,
        };

        (contracted, mapping)
    }
}

/// Louvain Community Detection (Full Louvain with hierarchical community aggregation)
///
/// Detects communities by greedily optimizing modularity. Implements both
/// Phase 1 (local modularity moves) and Phase 2 (community aggregation /
/// graph contraction), iterating until convergence or the iteration limit.
///
/// The `iterations` parameter limits the number of outer loops (Phase 1 + Phase 2 cycles).
/// The `resolution` parameter controls community granularity (higher = more communities).
///
/// Returns a map of NodeID -> CommunityID.
pub fn louvain(
    projection: &GraphProjection,
    iterations: usize,
    resolution: f64,
) -> HashMap<String, u32> {
    let node_count = projection.node_count();

    if node_count == 0 {
        return HashMap::new();
    }

    // Build initial internal graph from the projection
    let mut graph = InternalGraph::from_projection(projection);

    // Track the cumulative mapping from original nodes to current super-nodes.
    // membership[original_node] = current super-node index
    let mut membership: Vec<usize> = (0..node_count).collect();

    for _ in 0..iterations {
        // Phase 1: local moves on the current (possibly contracted) graph
        let (community, any_moved) = graph.phase1(iterations, resolution);

        if !any_moved {
            // No improvement -- converged
            break;
        }

        // Check if all nodes ended up in one community (nothing more to contract)
        let num_communities = community
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .len();
        if num_communities == graph.node_count {
            let mut renumber: HashMap<usize, usize> = HashMap::new();
            for &c in &community {
                let len = renumber.len();
                renumber.entry(c).or_insert(len);
            }
            for m in membership.iter_mut() {
                *m = *renumber.get(&community[*m]).unwrap();
            }
            break;
        }

        // Phase 2: contract the graph
        let (contracted, mapping) = graph.contract(&community);

        // Update the cumulative membership
        for m in membership.iter_mut() {
            *m = mapping[community[*m]];
        }

        // If the contracted graph has the same number of nodes, we're done
        if contracted.node_count == graph.node_count {
            break;
        }

        graph = contracted;
    }

    // Map results back to string IDs
    let mut comm_renumber: HashMap<usize, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut result = HashMap::with_capacity(node_count);

    for i in 0..node_count {
        if let Some(node_id) = projection.get_node_id(i as u32) {
            let comm = membership[i];
            let final_id = *comm_renumber.entry(comm).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
            result.insert(node_id.clone(), final_id);
        }
    }

    result
}

#[cfg(test)]
#[path = "louvain_tests.rs"]
mod tests;
