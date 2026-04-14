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
#[path = "yen_tests.rs"]
mod tests;
