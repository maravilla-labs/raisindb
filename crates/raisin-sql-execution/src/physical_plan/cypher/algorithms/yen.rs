//! Yen's K-Shortest Paths Algorithm
//!
//! Finds the K shortest loopless paths between two nodes.
//!
//! Time Complexity: O(K * V * (E + V log V))
//! Space Complexity: O(K * V)

use super::super::types::{PathInfo, RelationInfo};
use super::types::{GraphAdjacency, GraphNodeId, IndexedPath, WeightedIndexedPath};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// State for priority queue
#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f64,
    node_idx: usize,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.node_idx.cmp(&other.node_idx))
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Find K shortest paths between two nodes
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node
/// * `end` - Target node
/// * `k` - Number of paths to find
/// * `cost_fn` - Function to calculate edge cost
///
/// # Returns
/// * Vector of PathInfo objects sorted by cost
pub fn k_shortest_paths<C>(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    k: usize,
    cost_fn: C,
) -> Vec<PathInfo>
where
    C: Fn(&GraphNodeId, &GraphNodeId, &str) -> f64,
{
    if start == end || k == 0 {
        return vec![PathInfo::new(start.1.clone(), start.0.clone())];
    }

    // Map nodes to integers
    let mut node_to_idx: HashMap<(String, String), usize> = HashMap::new();
    let mut idx_to_node: Vec<(String, String)> = Vec::new();

    let mut all_nodes = HashSet::new();
    all_nodes.insert(start.clone());
    all_nodes.insert(end.clone());
    for (src, targets) in adjacency {
        all_nodes.insert(src.clone());
        for (tgt_w, tgt_id, _) in targets {
            all_nodes.insert((tgt_w.clone(), tgt_id.clone()));
        }
    }

    for (i, node) in all_nodes.into_iter().enumerate() {
        node_to_idx.insert(node.clone(), i);
        idx_to_node.push(node);
    }

    let start_idx = *node_to_idx
        .get(start)
        .expect("node must be in index map — invariant maintained by prior insert");
    let end_idx = *node_to_idx
        .get(end)
        .expect("node must be in index map — invariant maintained by prior insert");

    // Helper for Dijkstra
    let run_dijkstra = |start: usize,
                        end: usize,
                        excluded_edges: &HashSet<(usize, usize)>,
                        excluded_nodes: &HashSet<usize>|
     -> Option<WeightedIndexedPath> {
        let mut dist = vec![f64::INFINITY; idx_to_node.len()];
        let mut parent = HashMap::new();
        let mut heap = BinaryHeap::new();

        dist[start] = 0.0;
        heap.push(State {
            cost: 0.0,
            node_idx: start,
        });

        while let Some(State { cost, node_idx }) = heap.pop() {
            if cost > dist[node_idx] {
                continue;
            }
            if node_idx == end {
                break;
            }

            let u_node = &idx_to_node[node_idx];
            if let Some(neighbors) = adjacency.get(u_node) {
                for (v_w, v_id, rel_type) in neighbors {
                    let v_node = (v_w.clone(), v_id.clone());
                    if let Some(&v_idx) = node_to_idx.get(&v_node) {
                        if excluded_nodes.contains(&v_idx) {
                            continue;
                        }
                        if excluded_edges.contains(&(node_idx, v_idx)) {
                            continue;
                        }

                        let weight = cost_fn(u_node, &v_node, rel_type);
                        let next_cost = cost + weight;

                        if next_cost < dist[v_idx] {
                            dist[v_idx] = next_cost;
                            parent.insert(v_idx, (node_idx, rel_type.clone()));
                            heap.push(State {
                                cost: next_cost,
                                node_idx: v_idx,
                            });
                        }
                    }
                }
            }
        }

        if dist[end] == f64::INFINITY {
            return None;
        }

        // Reconstruct path
        let mut path = Vec::new();
        let mut curr = end;
        while curr != start {
            let (prev, rel) = parent
                .get(&curr)
                .expect("node must be in index map — invariant maintained by prior insert");
            path.push((*prev, curr, rel.clone()));
            curr = *prev;
        }
        path.reverse();
        Some((dist[end], path))
    };

    let mut accepted_paths: Vec<WeightedIndexedPath> = Vec::new();
    let mut potential_paths = BinaryHeap::new(); // Potential paths

    // 1. Find first shortest path
    if let Some((cost, path)) = run_dijkstra(start_idx, end_idx, &HashSet::new(), &HashSet::new()) {
        accepted_paths.push((cost, path));
    } else {
        return Vec::new();
    }

    // 2. Find k-1 more paths
    for k_curr in 1..k {
        if k_curr > accepted_paths.len() {
            break;
        }

        let prev_path = &accepted_paths[k_curr - 1].1;

        // Spur node ranges from start to second-to-last node of previous path
        for i in 0..prev_path.len() {
            let spur_node = prev_path[i].0;
            let root_path = &prev_path[0..i];

            let mut excluded_edges = HashSet::new();
            let mut excluded_nodes = HashSet::new();

            // Remove edges that are part of previous shortest paths which share the same root path
            for (_cost, p) in &accepted_paths {
                if i < p.len() && &p[0..i] == root_path {
                    let edge = (p[i].0, p[i].1);
                    excluded_edges.insert(edge);
                }
            }

            // Remove nodes in root path from graph (except spur node)
            for edge in root_path {
                excluded_nodes.insert(edge.0);
            }

            // Calculate spur path from spur_node to end
            if let Some((_spur_cost, spur_path)) =
                run_dijkstra(spur_node, end_idx, &excluded_edges, &excluded_nodes)
            {
                // Total path = root_path + spur_path
                let mut total_path = root_path.to_vec();
                total_path.extend(spur_path);

                // Calculate total cost
                let mut total_cost = 0.0;
                for (u, v, rel) in &total_path {
                    let u_node = &idx_to_node[*u];
                    let v_node = &idx_to_node[*v];
                    total_cost += cost_fn(u_node, v_node, rel);
                }

                // Add to B (potential paths)
                // We use negative cost for max-heap to behave as min-heap for sorting
                // But here we want to store candidates.
                // Let's wrap in a struct that implements Ord correctly for min-heap
                // Actually B should be a min-heap of candidates? No, we want the best candidate.
                // Standard Yen's uses a min-heap to extract the best candidate.
                potential_paths.push(PathCandidate {
                    cost: total_cost,
                    path: total_path,
                });
            }
        }

        if potential_paths.is_empty() {
            break;
        }

        // Move best path from B to A
        // B is a max-heap by default. We need min-heap behavior.
        // Let's invert the ordering in PathCandidate.
        let best = potential_paths.pop().unwrap();

        // Check if path already in A (Yen's can generate duplicates)
        let is_duplicate = accepted_paths.iter().any(|(_, p)| p == &best.path);
        if !is_duplicate {
            accepted_paths.push((best.cost, best.path));
        } else {
            // If duplicate, try next best
            // In a real implementation we should use a set for B to avoid duplicates early
            // or just pop until non-duplicate.
            while let Some(next) = potential_paths.pop() {
                if !accepted_paths.iter().any(|(_, p)| p == &next.path) {
                    accepted_paths.push((next.cost, next.path));
                    break;
                }
            }
        }
    }

    // Convert A to PathInfo objects
    accepted_paths
        .into_iter()
        .map(|(_cost, path_edges)| {
            let mut path = PathInfo::new(start.1.clone(), start.0.clone());
            for (_, v_idx, rel_type) in path_edges {
                let v_node = &idx_to_node[v_idx];
                let rel_info = RelationInfo {
                    source_var: String::new(),
                    target_var: String::new(),
                    relation_type: rel_type,
                    properties: HashMap::new(),
                };
                path = path.extend(rel_info, v_node.1.clone(), v_node.0.clone());
            }
            path
        })
        .collect()
}

#[derive(Clone, PartialEq)]
struct PathCandidate {
    cost: f64,
    path: IndexedPath,
}

impl Eq for PathCandidate {}

impl Ord for PathCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for Min-Heap behavior in BinaryHeap
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for PathCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();
        // A -> B -> D (cost 2)
        // A -> C -> D (cost 2)
        // A -> B -> C -> D (cost 3)

        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![
                ("ws".to_string(), "B".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![
                ("ws".to_string(), "D".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_k_shortest_paths() {
        let graph = create_test_graph();
        let start = ("ws".to_string(), "A".to_string());
        let end = ("ws".to_string(), "D".to_string());

        let paths = k_shortest_paths(&graph, &start, &end, 3, |_, _, _| 1.0);

        assert_eq!(paths.len(), 3);
        // Lengths should be 2, 2, 3 (in terms of hops/cost)
        // PathInfo length is number of hops
        assert_eq!(paths[0].length, 2);
        assert_eq!(paths[1].length, 2);
        assert_eq!(paths[2].length, 3);
    }
}
