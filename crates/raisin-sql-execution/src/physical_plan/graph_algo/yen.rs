//! Yen's K-Shortest Paths.
//!
//! Moved here from `physical_plan/cypher/algorithms/yen.rs`.
//!
//! Finds the K shortest loopless paths between two nodes.
//!
//! Time complexity: O(K * V * (E + V log V)); space: O(K * V).
//!
//! Note: the PGQ `SHORTEST k` selector is DEFERRED, not wired to this yet —
//! it returns k paths per `(start, end)` pair, which multiplies one binding
//! into k rows, and that row-multiplication has to be specified against
//! `ORDER BY` / `LIMIT` first. See `docs/OPEN-ITEMS.md`.

use super::path::GraphPath;
use super::types::{GraphAdjacency, GraphEdge, GraphNodeId, IndexedPath, WeightedIndexedPath};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Min-heap entry for Dijkstra's priority queue.
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

/// One candidate path in Yen's B-set, ordered cheapest-first.
#[derive(Clone, PartialEq)]
struct PathCandidate {
    cost: f64,
    path: IndexedPath,
}

impl Eq for PathCandidate {}

impl Ord for PathCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed so `BinaryHeap` yields the cheapest candidate first.
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

/// Find the K shortest loopless paths between two nodes.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node
/// * `end` - Target node
/// * `k` - Number of paths to find
/// * `cost_fn` - Per-edge cost: `(source, target, relation_type) -> cost`
///
/// # Returns
/// Paths sorted by total cost, ascending.
pub fn k_shortest_paths<C>(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    k: usize,
    cost_fn: C,
) -> Vec<GraphPath>
where
    C: Fn(&GraphNodeId, &GraphNodeId, &str) -> f64,
{
    if start == end || k == 0 {
        return vec![GraphPath::from_key(start)];
    }

    let mut node_to_idx: HashMap<GraphNodeId, usize> = HashMap::new();
    let mut idx_to_node: Vec<GraphNodeId> = Vec::new();
    // Edge lookup so a reconstructed path keeps the target node type and weight.
    let mut edge_by_hop: HashMap<(usize, usize, String), GraphEdge> = HashMap::new();

    let mut all_nodes = HashSet::new();
    all_nodes.insert(start.clone());
    all_nodes.insert(end.clone());
    for (src, targets) in adjacency {
        all_nodes.insert(src.clone());
        for edge in targets {
            all_nodes.insert(edge.target());
        }
    }

    for (i, node) in all_nodes.into_iter().enumerate() {
        node_to_idx.insert(node.clone(), i);
        idx_to_node.push(node);
    }

    for (src, targets) in adjacency {
        let Some(&src_idx) = node_to_idx.get(src) else {
            continue;
        };
        for edge in targets {
            if let Some(&tgt_idx) = node_to_idx.get(&edge.target()) {
                edge_by_hop
                    .entry((src_idx, tgt_idx, edge.relation_type.clone()))
                    .or_insert_with(|| edge.clone());
            }
        }
    }

    let (Some(&start_idx), Some(&end_idx)) = (node_to_idx.get(start), node_to_idx.get(end)) else {
        return Vec::new();
    };

    let run_dijkstra = |from: usize,
                        to: usize,
                        excluded_edges: &HashSet<(usize, usize)>,
                        excluded_nodes: &HashSet<usize>|
     -> Option<WeightedIndexedPath> {
        let mut dist = vec![f64::INFINITY; idx_to_node.len()];
        let mut parent: HashMap<usize, (usize, String)> = HashMap::new();
        let mut heap = BinaryHeap::new();

        dist[from] = 0.0;
        heap.push(State {
            cost: 0.0,
            node_idx: from,
        });

        while let Some(State { cost, node_idx }) = heap.pop() {
            if cost > dist[node_idx] {
                continue;
            }
            if node_idx == to {
                break;
            }

            let u_node = &idx_to_node[node_idx];
            let Some(neighbors) = adjacency.get(u_node) else {
                continue;
            };
            for edge in neighbors {
                let v_node = edge.target();
                let Some(&v_idx) = node_to_idx.get(&v_node) else {
                    continue;
                };
                if excluded_nodes.contains(&v_idx) || excluded_edges.contains(&(node_idx, v_idx)) {
                    continue;
                }

                let next_cost = cost + cost_fn(u_node, &v_node, &edge.relation_type);
                if next_cost < dist[v_idx] {
                    dist[v_idx] = next_cost;
                    parent.insert(v_idx, (node_idx, edge.relation_type.clone()));
                    heap.push(State {
                        cost: next_cost,
                        node_idx: v_idx,
                    });
                }
            }
        }

        if dist[to] == f64::INFINITY {
            return None;
        }

        let mut path = Vec::new();
        let mut curr = to;
        while curr != from {
            let (prev, rel) = parent.get(&curr)?;
            path.push((*prev, curr, rel.clone()));
            curr = *prev;
        }
        path.reverse();
        Some((dist[to], path))
    };

    let mut accepted: Vec<WeightedIndexedPath> = Vec::new();
    let mut candidates: BinaryHeap<PathCandidate> = BinaryHeap::new();

    match run_dijkstra(start_idx, end_idx, &HashSet::new(), &HashSet::new()) {
        Some(first) => accepted.push(first),
        None => return Vec::new(),
    }

    for k_curr in 1..k {
        if k_curr > accepted.len() {
            break;
        }

        let prev_path = accepted[k_curr - 1].1.clone();

        for i in 0..prev_path.len() {
            let spur_node = prev_path[i].0;
            let root_path = &prev_path[0..i];

            let mut excluded_edges = HashSet::new();
            let mut excluded_nodes = HashSet::new();

            for (_cost, p) in &accepted {
                if i < p.len() && &p[0..i] == root_path {
                    excluded_edges.insert((p[i].0, p[i].1));
                }
            }
            for edge in root_path {
                excluded_nodes.insert(edge.0);
            }

            if let Some((_spur_cost, spur_path)) =
                run_dijkstra(spur_node, end_idx, &excluded_edges, &excluded_nodes)
            {
                let mut total_path = root_path.to_vec();
                total_path.extend(spur_path);

                let total_cost = total_path
                    .iter()
                    .map(|(u, v, rel)| cost_fn(&idx_to_node[*u], &idx_to_node[*v], rel))
                    .sum();

                candidates.push(PathCandidate {
                    cost: total_cost,
                    path: total_path,
                });
            }
        }

        // Pop the cheapest candidate that is not already accepted; Yen's can
        // regenerate a path it has already emitted.
        let mut promoted = false;
        while let Some(next) = candidates.pop() {
            if !accepted.iter().any(|(_, p)| p == &next.path) {
                accepted.push((next.cost, next.path));
                promoted = true;
                break;
            }
        }
        if !promoted {
            break;
        }
    }

    accepted
        .into_iter()
        .map(|(_cost, hops)| {
            let mut path = GraphPath::from_key(start);
            for (u_idx, v_idx, rel_type) in hops {
                let edge = edge_by_hop
                    .get(&(u_idx, v_idx, rel_type.clone()))
                    .cloned()
                    .unwrap_or_else(|| {
                        let target = &idx_to_node[v_idx];
                        GraphEdge::untyped(target.0.clone(), target.1.clone(), rel_type)
                    });
                path.push(&edge);
            }
            path
        })
        .collect()
}
