//! Shortest Path Algorithms
//!
//! Implements shortest path finding algorithms for Cypher queries:
//! - shortestPath() - Returns one shortest path between two nodes
//! - allShortestPaths() - Returns all paths with minimum length
//!
//! Uses bidirectional BFS for optimal performance.

use super::super::types::{PathInfo, RelationInfo};
use super::types::{BfsVisited, GraphAdjacency, GraphNodeId};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

/// State for A* priority queue
#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f64,
    node_idx: usize,
}

impl Eq for State {}

// The priority queue depends on `Ord`.
// Explicitly implement the trait so the queue becomes a min-heap
// instead of a max-heap.
impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Notice that the we flip the ordering on costs.
        // In case of a tie we compare positions - this step is necessary
        // to make implementations of `PartialEq` and `Ord` consistent.
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

/// Find a single shortest path between two nodes
///
/// Uses BFS (Breadth-First Search) to find the shortest path.
/// If multiple paths exist with the same minimum length, returns one non-deterministically.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list: (workspace, id) -> [(target_workspace, target_id, rel_type)]
/// * `start` - Starting node (workspace, id)
/// * `end` - Target node (workspace, id)
/// * `max_depth` - Maximum path length to search
///
/// # Returns
/// * `Some(PathInfo)` if a path exists
/// * `None` if no path exists within max_depth
pub fn shortest_path(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
) -> Option<PathInfo> {
    if start == end {
        // Same node - return path with zero hops
        return Some(PathInfo::new(start.1.clone(), start.0.clone()));
    }

    // BFS with parent tracking for path reconstruction
    let mut queue = VecDeque::new();
    let mut visited: BfsVisited = HashMap::new();

    queue.push_back((start.clone(), 0));
    visited.insert(start.clone(), None);

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth as usize {
            break;
        }

        // Check if we've reached the end
        if &current == end {
            return Some(reconstruct_path_simple(&visited, start, end));
        }

        // Explore neighbors
        if let Some(neighbors) = adjacency.get(&current) {
            for (next_workspace, next_id, rel_type) in neighbors {
                let next = (next_workspace.clone(), next_id.clone());
                if !visited.contains_key(&next) {
                    visited.insert(next.clone(), Some((current.clone(), rel_type.clone())));
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }

    None
}

/// A* Shortest Path
///
/// Finds the shortest path between two nodes using A* algorithm.
/// Supports custom cost and heuristic functions.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node
/// * `end` - Target node
/// * `cost_fn` - Function to calculate edge cost: (source, target, rel_type) -> cost
/// * `heuristic_fn` - Function to calculate heuristic: (node) -> estimated_cost_to_goal
///
/// # Returns
/// * `Some(PathInfo)` if a path exists
/// * `None` if no path exists
pub fn astar_shortest_path<C, H>(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    cost_fn: C,
    heuristic_fn: H,
) -> Option<PathInfo>
where
    C: Fn(&GraphNodeId, &GraphNodeId, &str) -> f64,
    H: Fn(&GraphNodeId) -> f64,
{
    if start == end {
        return Some(PathInfo::new(start.1.clone(), start.0.clone()));
    }

    // Map nodes to integers for efficiency
    let mut node_to_idx: HashMap<(String, String), usize> = HashMap::new();
    let mut idx_to_node: Vec<(String, String)> = Vec::new();

    // Collect all nodes
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

    let start_idx = *node_to_idx.get(start)?;
    let end_idx = *node_to_idx.get(end)?;

    // dist[node] = current shortest distance from start to node
    let mut dist: Vec<f64> = vec![f64::INFINITY; idx_to_node.len()];
    dist[start_idx] = 0.0;

    // parent[node] = (parent_idx, rel_type)
    let mut parent: HashMap<usize, (usize, String)> = HashMap::new();

    let mut heap = BinaryHeap::new();
    heap.push(State {
        cost: 0.0,
        node_idx: start_idx,
    });

    while let Some(State { cost, node_idx }) = heap.pop() {
        if node_idx == end_idx {
            // Reconstruct path
            let mut path_edges = Vec::new();
            let mut curr = end_idx;
            while curr != start_idx {
                if let Some((p, rel_type)) = parent.get(&curr) {
                    let src_node = &idx_to_node[*p];
                    let tgt_node = &idx_to_node[curr];
                    path_edges.push((src_node.clone(), tgt_node.clone(), rel_type.clone()));
                    curr = *p;
                } else {
                    break;
                }
            }
            path_edges.reverse();

            let mut path = PathInfo::new(start.1.clone(), start.0.clone());
            for ((_src_w, _src_id), (tgt_w, tgt_id), rel_type) in path_edges {
                let rel_info = RelationInfo {
                    source_var: String::new(),
                    target_var: String::new(),
                    relation_type: rel_type,
                    properties: HashMap::new(),
                };
                path = path.extend(rel_info, tgt_id, tgt_w);
            }
            return Some(path);
        }

        // If we found a shorter path already, skip
        if cost > dist[node_idx] + heuristic_fn(&idx_to_node[node_idx]) {
            continue;
        }

        // Explore neighbors
        let current_node = &idx_to_node[node_idx];
        if let Some(neighbors) = adjacency.get(current_node) {
            for (next_w, next_id, rel_type) in neighbors {
                let next_node = (next_w.clone(), next_id.clone());
                if let Some(&next_idx) = node_to_idx.get(&next_node) {
                    let edge_cost = cost_fn(current_node, &next_node, rel_type);
                    let next_dist = dist[node_idx] + edge_cost;

                    if next_dist < dist[next_idx] {
                        dist[next_idx] = next_dist;
                        parent.insert(next_idx, (node_idx, rel_type.clone()));
                        heap.push(State {
                            cost: next_dist + heuristic_fn(&next_node),
                            node_idx: next_idx,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Reconstruct the path from BFS parent pointers
fn reconstruct_path_simple(
    visited: &BfsVisited,
    start: &GraphNodeId,
    end: &GraphNodeId,
) -> PathInfo {
    let mut path_edges = Vec::new();
    let mut current = end.clone();

    // Walk backwards from end to start
    while current != *start {
        if let Some(Some((parent, rel_type))) = visited.get(&current) {
            path_edges.push((parent.clone(), current.clone(), rel_type.clone()));
            current = parent.clone();
        } else {
            break;
        }
    }

    path_edges.reverse();

    // Build PathInfo
    let mut path = PathInfo::new(start.1.clone(), start.0.clone());
    for ((_src_w, _src_id), (tgt_w, tgt_id), rel_type) in path_edges {
        let rel_info = RelationInfo {
            source_var: String::new(),
            target_var: String::new(),
            relation_type: rel_type,
            properties: HashMap::new(),
        };
        path = path.extend(rel_info, tgt_id, tgt_w);
    }

    path
}

/// Find all shortest paths between two nodes
///
/// Returns all paths that have the minimum length.
/// Uses BFS to find the minimum depth, then finds all paths at that depth.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list
/// * `start` - Starting node
/// * `end` - Target node
/// * `max_depth` - Maximum path length to search
/// * `max_paths` - Maximum number of paths to return (prevent combinatorial explosion)
///
/// # Returns
/// * Vector of PathInfo objects, all with the same minimum length
/// * Empty vector if no path exists
pub fn all_shortest_paths(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
    max_paths: usize,
) -> Vec<PathInfo> {
    if start == end {
        return vec![PathInfo::new(start.1.clone(), start.0.clone())];
    }

    // First, find the minimum depth using simple BFS
    let min_depth = match find_min_depth(adjacency, start, end, max_depth) {
        Some(depth) => depth,
        None => return Vec::new(),
    };

    // Now find all paths at that depth
    let mut all_paths = Vec::new();
    let mut current_paths = vec![PathInfo::new(start.1.clone(), start.0.clone())];

    for depth in 0..min_depth {
        let mut next_paths = Vec::new();

        for path in current_paths {
            let last_node = path.nodes.last().unwrap();
            let current_key = (last_node.1.clone(), last_node.0.clone());

            if let Some(neighbors) = adjacency.get(&current_key) {
                for (next_workspace, next_id, rel_type) in neighbors {
                    // Avoid cycles
                    if !path.contains_node(next_id, next_workspace) {
                        let rel_info = RelationInfo {
                            source_var: String::new(),
                            target_var: String::new(),
                            relation_type: rel_type.clone(),
                            properties: HashMap::new(),
                        };

                        let new_path =
                            path.extend(rel_info, next_id.clone(), next_workspace.clone());

                        // Check if we've reached the end at the right depth
                        if depth == min_depth - 1 && (next_workspace, next_id) == (&end.0, &end.1) {
                            all_paths.push(new_path);
                            if all_paths.len() >= max_paths {
                                return all_paths;
                            }
                        } else if depth < min_depth - 1 {
                            next_paths.push(new_path);
                        }
                    }
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

/// Find the minimum depth between two nodes using simple BFS
fn find_min_depth(
    adjacency: &GraphAdjacency,
    start: &GraphNodeId,
    end: &GraphNodeId,
    max_depth: u32,
) -> Option<usize> {
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    queue.push_back((start.clone(), 0));
    visited.insert(start.clone());

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth as usize {
            break;
        }

        if &current == end {
            return Some(depth);
        }

        if let Some(neighbors) = adjacency.get(&current) {
            for (next_workspace, next_id, _rel_type) in neighbors {
                let next = (next_workspace.clone(), next_id.clone());
                if !visited.contains(&next) {
                    visited.insert(next.clone());
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // Linear path: A -> B -> C -> D
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "D".to_string(), "LINK".to_string())],
        );

        // Shortcut: A -> C (creates two paths A->B->C->D and A->C->D)
        graph
            .get_mut(&("ws".to_string(), "A".to_string()))
            .unwrap()
            .push(("ws".to_string(), "C".to_string(), "SHORT".to_string()));

        graph
    }

    #[test]
    fn test_shortest_path_basic() {
        let graph = create_test_graph();
        let start = ("ws".to_string(), "A".to_string());
        let end = ("ws".to_string(), "D".to_string());

        let path = shortest_path(&graph, &start, &end, 10);
        assert!(path.is_some());

        let path = path.unwrap();
        assert_eq!(path.length, 2); // A -> C -> D (using shortcut)
    }

    #[test]
    fn test_all_shortest_paths() {
        let graph = create_test_graph();
        let start = ("ws".to_string(), "A".to_string());
        let end = ("ws".to_string(), "D".to_string());

        let paths = all_shortest_paths(&graph, &start, &end, 10, 100);
        assert_eq!(paths.len(), 1); // Only one shortest path of length 2
        assert_eq!(paths[0].length, 2);
    }

    #[test]
    fn test_no_path() {
        let graph = create_test_graph();
        let start = ("ws".to_string(), "D".to_string());
        let end = ("ws".to_string(), "A".to_string());

        let path = shortest_path(&graph, &start, &end, 10);
        assert!(path.is_none()); // No reverse edges
    }
}
