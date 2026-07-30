//! Hop-count shortest path search (BFS family).
//!
//! Moved here from `physical_plan/cypher/algorithms/shortest_path.rs` so the
//! Cypher and SQL/PGQ engines call ONE copy. Cypher keeps its old import
//! paths via re-export shims in `cypher::algorithms`.
//!
//! - [`shortest_path`] — one minimum-hop path (`ANY SHORTEST`)
//! - [`all_shortest_paths`] — every path of minimum length (`ALL SHORTEST`)
//!
//! Both are node-distinct (ACYCLIC) in effect: BFS never revisits a node, and
//! `all_shortest_paths` guards each extension with `contains_node`.

use super::path::GraphPath;
use super::types::{GraphAdjacency, GraphEdge, GraphNodeId};
use std::collections::{HashMap, HashSet, VecDeque};

/// BFS parent pointers rich enough to rebuild a full [`GraphPath`].
///
/// The plain `BfsVisited` alias stores only the relation type, which is not
/// enough to recover the target node type or the edge weight.
type ParentMap = HashMap<GraphNodeId, Option<(GraphNodeId, GraphEdge)>>;

/// Find a single minimum-hop path between two nodes.
///
/// Uses BFS with parent tracking. If several paths share the minimum length,
/// one of them is returned; which one is not specified.
///
/// The returned path's start node carries an empty `node_type` — the caller
/// holds the binding that knows it and can supply it with
/// [`GraphPath::with_start_node_type`].
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node `(workspace, id)`
/// * `end` - Target node `(workspace, id)`
/// * `max_depth` - Maximum path length to search
pub fn shortest_path(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
) -> Option<GraphPath> {
    if start == end {
        return Some(GraphPath::from_key(start));
    }

    let mut queue = VecDeque::new();
    let mut visited: ParentMap = HashMap::new();

    queue.push_back((start.clone(), 0usize));
    visited.insert(start.clone(), None);

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth as usize {
            break;
        }

        if &current == end {
            return Some(reconstruct_path(&visited, start, end));
        }

        if let Some(neighbors) = adjacency.get(&current) {
            for edge in neighbors {
                let next = edge.target();
                if !visited.contains_key(&next) {
                    visited.insert(next.clone(), Some((current.clone(), edge.clone())));
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }

    None
}

/// Rebuild an ordered path from BFS parent pointers.
fn reconstruct_path(visited: &ParentMap, start: &GraphNodeId, end: &GraphNodeId) -> GraphPath {
    let mut hops: Vec<GraphEdge> = Vec::new();
    let mut current = end.clone();

    while current != *start {
        match visited.get(&current) {
            Some(Some((parent, edge))) => {
                hops.push(edge.clone());
                current = parent.clone();
            }
            _ => break,
        }
    }

    hops.reverse();

    let mut path = GraphPath::from_key(start);
    for edge in &hops {
        path.push(edge);
    }
    path
}

/// Find every path of minimum length between two nodes.
///
/// Returns all paths that tie for the minimum hop count, capped at
/// `max_paths` to keep a combinatorial graph from exploding.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node
/// * `end` - Target node
/// * `max_depth` - Maximum path length to search
/// * `max_paths` - Maximum number of paths to return
pub fn all_shortest_paths(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
    max_paths: usize,
) -> Vec<GraphPath> {
    if start == end {
        return vec![GraphPath::from_key(start)];
    }

    let Some(min_depth) = find_min_depth(adjacency, start, end, max_depth) else {
        return Vec::new();
    };

    let mut all_paths = Vec::new();
    let mut current_paths = vec![GraphPath::from_key(start)];

    for depth in 0..min_depth {
        let mut next_paths = Vec::new();

        for path in current_paths {
            let Some(last) = path.last() else { continue };
            let current_key = last.key();

            let Some(neighbors) = adjacency.get(&current_key) else {
                continue;
            };

            for edge in neighbors {
                // Node-distinct: never revisit a node already on the path.
                if path.contains_key(&edge.target()) {
                    continue;
                }

                let extended = path.extended(edge);
                let reached_end = (&edge.target_workspace, &edge.target_id) == (&end.0, &end.1);

                if depth == min_depth - 1 && reached_end {
                    all_paths.push(extended);
                    if all_paths.len() >= max_paths {
                        return all_paths;
                    }
                } else if depth < min_depth - 1 {
                    next_paths.push(extended);
                }
            }
        }

        current_paths = next_paths;
        if current_paths.is_empty() {
            break;
        }
    }

    all_paths
}

/// Minimum hop count between two nodes, or `None` if unreachable.
fn find_min_depth(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
) -> Option<usize> {
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    queue.push_back((start.clone(), 0usize));
    visited.insert(start.clone());

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth as usize {
            break;
        }

        if &current == end {
            return Some(depth);
        }

        if let Some(neighbors) = adjacency.get(&current) {
            for edge in neighbors {
                let next = edge.target();
                if visited.insert(next.clone()) {
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }

    None
}
