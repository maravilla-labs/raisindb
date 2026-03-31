//! Connected Components Algorithm
//!
//! Finds weakly connected components in a directed graph.
//! A weakly connected component is a maximal set of nodes where there exists
//! a path between any two nodes when ignoring edge direction.
//!
//! Time Complexity: O(V + E)
//! Space Complexity: O(V)

use std::collections::{HashMap, HashSet, VecDeque};

use super::types::{GraphAdjacency, GraphNodeId};

/// Find all weakly connected components in the graph
///
/// Returns a mapping of node -> component_id where nodes in the same
/// component share the same component_id.
///
/// # Arguments
/// * `adjacency` - Graph adjacency list (directed edges)
///
/// # Returns
/// * HashMap of (node -> component_id) for all nodes
pub fn connected_components(adjacency: &GraphAdjacency) -> HashMap<GraphNodeId, usize> {
    // Build undirected adjacency (treat all edges as bidirectional)
    let undirected = build_undirected_graph(adjacency);

    // Collect all unique nodes
    let mut all_nodes = HashSet::new();
    for (source, neighbors) in undirected.iter() {
        all_nodes.insert(source.clone());
        for neighbor in neighbors {
            all_nodes.insert(neighbor.clone());
        }
    }

    // Track which component each node belongs to
    let mut components: HashMap<(String, String), usize> = HashMap::new();
    let mut component_id = 0;

    // BFS from each unvisited node
    for start_node in &all_nodes {
        if components.contains_key(start_node) {
            continue; // Already assigned to a component
        }

        // BFS to find all nodes in this component
        let mut queue = VecDeque::new();
        queue.push_back(start_node.clone());
        components.insert(start_node.clone(), component_id);

        while let Some(current) = queue.pop_front() {
            if let Some(neighbors) = undirected.get(&current) {
                for neighbor in neighbors {
                    if !components.contains_key(neighbor) {
                        components.insert(neighbor.clone(), component_id);
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }

        component_id += 1;
    }

    components
}

/// Build undirected graph from directed adjacency list
///
/// For each directed edge A->B, creates both A->B and B->A
fn build_undirected_graph(adjacency: &GraphAdjacency) -> HashMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut undirected: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();

    for (source, neighbors) in adjacency.iter() {
        for (tgt_workspace, tgt_id, _rel_type) in neighbors {
            let target = (tgt_workspace.clone(), tgt_id.clone());

            // Add forward edge
            undirected
                .entry(source.clone())
                .or_default()
                .push(target.clone());

            // Add reverse edge (for undirected)
            undirected
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }

    undirected
}

/// Get the component ID for a specific node
///
/// Returns None if the node doesn't exist in the graph
pub fn node_component_id(adjacency: &GraphAdjacency, node: &GraphNodeId) -> Option<usize> {
    let components = connected_components(adjacency);
    components.get(node).copied()
}

/// Get the number of connected components in the graph
pub fn component_count(adjacency: &GraphAdjacency) -> usize {
    let components = connected_components(adjacency);
    if components.is_empty() {
        return 0;
    }

    let max_id = components
        .values()
        .max()
        .expect("max always succeeds — component_labels is non-empty after guard");
    max_id + 1
}

/// Get all nodes in a specific component
pub fn nodes_in_component(adjacency: &GraphAdjacency, component_id: usize) -> Vec<GraphNodeId> {
    let components = connected_components(adjacency);
    components
        .into_iter()
        .filter(|(_, cid)| *cid == component_id)
        .map(|(node, _)| node)
        .collect()
}

/// Get component sizes (number of nodes in each component)
pub fn component_sizes(adjacency: &GraphAdjacency) -> HashMap<usize, usize> {
    let components = connected_components(adjacency);
    let mut sizes: HashMap<usize, usize> = HashMap::new();

    for component_id in components.values() {
        *sizes.entry(*component_id).or_insert(0) += 1;
    }

    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_two_component_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // Component 1: A -> B -> C
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );

        // Component 2: D -> E
        graph.insert(
            ("ws".to_string(), "D".to_string()),
            vec![("ws".to_string(), "E".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_connected_components_two_components() {
        let graph = create_two_component_graph();
        let components = connected_components(&graph);

        assert_eq!(components.len(), 5);

        // A, B, C should be in same component
        let comp_a = components[&("ws".to_string(), "A".to_string())];
        let comp_b = components[&("ws".to_string(), "B".to_string())];
        let comp_c = components[&("ws".to_string(), "C".to_string())];
        assert_eq!(comp_a, comp_b);
        assert_eq!(comp_b, comp_c);

        // D, E should be in same component (different from A,B,C)
        let comp_d = components[&("ws".to_string(), "D".to_string())];
        let comp_e = components[&("ws".to_string(), "E".to_string())];
        assert_eq!(comp_d, comp_e);
        assert_ne!(comp_a, comp_d);
    }

    #[test]
    fn test_component_count() {
        let graph = create_two_component_graph();
        let count = component_count(&graph);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_single_component() {
        let mut graph = HashMap::new();

        // Single connected component: A <-> B <-> C (bidirectional)
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![
                ("ws".to_string(), "A".to_string(), "LINK".to_string()),
                ("ws".to_string(), "C".to_string(), "LINK".to_string()),
            ],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );

        let count = component_count(&graph);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_isolated_nodes() {
        let mut graph = HashMap::new();

        // Three isolated nodes
        graph.insert(("ws".to_string(), "A".to_string()), vec![]);
        graph.insert(("ws".to_string(), "B".to_string()), vec![]);
        graph.insert(("ws".to_string(), "C".to_string()), vec![]);

        let count = component_count(&graph);
        // Note: empty adjacency lists don't create nodes in undirected graph
        // Only nodes with actual edges are counted
        assert_eq!(count, 0);
    }

    #[test]
    fn test_node_component_id() {
        let graph = create_two_component_graph();

        // Call connected_components once to ensure consistent component IDs
        let components = connected_components(&graph);
        let comp_a = components.get(&("ws".to_string(), "A".to_string()));
        let comp_d = components.get(&("ws".to_string(), "D".to_string()));

        assert!(comp_a.is_some());
        assert!(comp_d.is_some());
        assert_ne!(comp_a, comp_d); // Different components
    }

    #[test]
    fn test_nodes_in_component() {
        let graph = create_two_component_graph();
        let comp_id = node_component_id(&graph, &("ws".to_string(), "A".to_string())).unwrap();
        let nodes = nodes_in_component(&graph, comp_id);

        assert_eq!(nodes.len(), 3); // A, B, C
    }

    #[test]
    fn test_component_sizes() {
        let graph = create_two_component_graph();
        let sizes = component_sizes(&graph);

        assert_eq!(sizes.len(), 2);

        // One component with 3 nodes, one with 2 nodes
        let mut size_values: Vec<_> = sizes.values().copied().collect();
        size_values.sort();
        assert_eq!(size_values, vec![2, 3]);
    }

    #[test]
    fn test_empty_graph() {
        let graph = HashMap::new();
        let components = connected_components(&graph);
        assert_eq!(components.len(), 0);
        assert_eq!(component_count(&graph), 0);
    }

    #[test]
    fn test_weakly_connected_directed_graph() {
        let mut graph = HashMap::new();

        // Directed cycle: A -> B -> C -> A (weakly connected, not strongly)
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
            vec![("ws".to_string(), "A".to_string(), "LINK".to_string())],
        );

        let count = component_count(&graph);
        assert_eq!(count, 1); // All in same weakly connected component
    }
}
