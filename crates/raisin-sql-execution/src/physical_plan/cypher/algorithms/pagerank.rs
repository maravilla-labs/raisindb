//! PageRank Algorithm
//!
//! Implements the PageRank algorithm for measuring node influence in a graph.
//! Uses power iteration method with configurable damping factor and convergence criteria.
//!
//! Reference: Page, L., Brin, S., Motwani, R., & Winograd, T. (1999).
//! The PageRank citation ranking: Bringing order to the web.

use std::collections::{HashMap, HashSet};

use super::types::{GraphAdjacency, GraphNodeId};

/// Configuration for PageRank algorithm
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor (probability of following a link)
    /// Typically 0.85, meaning 85% chance of following links, 15% random jump
    pub damping_factor: f64,

    /// Maximum number of iterations
    pub max_iterations: usize,

    /// Convergence threshold - stop when max change < threshold
    pub convergence_threshold: f64,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping_factor: 0.85,
            max_iterations: 100,
            convergence_threshold: 0.0001,
        }
    }
}

/// Calculate PageRank for all nodes in the graph
///
/// Uses the power iteration algorithm:
/// PR(v) = (1-d)/N + d * Σ(PR(u)/L(u))
///
/// Where:
/// - PR(v) = PageRank of node v
/// - d = damping factor
/// - N = total number of nodes
/// - u = nodes linking to v
/// - L(u) = number of outgoing links from u
///
/// # Arguments
/// * `adjacency` - Graph adjacency list: (workspace, id) -> [(target_workspace, target_id, rel_type)]
/// * `config` - PageRank configuration (damping, iterations, convergence)
///
/// # Returns
/// * HashMap of (workspace, id) -> PageRank score
pub fn pagerank(adjacency: &GraphAdjacency, config: &PageRankConfig) -> HashMap<GraphNodeId, f64> {
    // Collect all nodes in the graph
    let mut all_nodes = HashSet::new();
    for (source, targets) in adjacency.iter() {
        all_nodes.insert(source.clone());
        for (tgt_workspace, tgt_id, _) in targets {
            all_nodes.insert((tgt_workspace.clone(), tgt_id.clone()));
        }
    }

    let num_nodes = all_nodes.len();
    if num_nodes == 0 {
        return HashMap::new();
    }

    let initial_pr = 1.0 / num_nodes as f64;

    // Initialize PageRank scores
    let mut pagerank: HashMap<(String, String), f64> = all_nodes
        .iter()
        .map(|node| (node.clone(), initial_pr))
        .collect();

    // Build reverse adjacency (incoming links)
    let mut incoming: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for (source, targets) in adjacency.iter() {
        for (tgt_workspace, tgt_id, _) in targets {
            let target = (tgt_workspace.clone(), tgt_id.clone());
            incoming.entry(target).or_default().push(source.clone());
        }
    }

    // Calculate outgoing link counts
    let out_degree: HashMap<(String, String), usize> = adjacency
        .iter()
        .map(|(node, targets)| (node.clone(), targets.len()))
        .collect();

    // Power iteration
    for iteration in 0..config.max_iterations {
        let mut new_pagerank = HashMap::new();
        let mut max_change = 0.0;

        // Calculate new PageRank for each node
        for node in all_nodes.iter() {
            // Base probability: random jump to any page
            let mut new_pr = (1.0 - config.damping_factor) / num_nodes as f64;

            // Add contributions from incoming links
            if let Some(incoming_nodes) = incoming.get(node) {
                for source in incoming_nodes {
                    let source_pr = pagerank.get(source).copied().unwrap_or(initial_pr);
                    let source_out_degree = out_degree.get(source).copied().unwrap_or(1);

                    // Contribution = PR(source) / outgoing_links(source)
                    new_pr += config.damping_factor * (source_pr / source_out_degree as f64);
                }
            }

            // Handle dangling nodes (nodes with no outgoing links)
            // Distribute their PageRank equally to all nodes
            let dangling_contribution: f64 = all_nodes
                .iter()
                .filter(|n| !adjacency.contains_key(*n))
                .map(|n| pagerank.get(n).copied().unwrap_or(initial_pr))
                .sum();

            if dangling_contribution > 0.0 {
                new_pr += config.damping_factor * (dangling_contribution / num_nodes as f64);
            }

            // Track maximum change for convergence
            let old_pr = pagerank.get(node).copied().unwrap_or(initial_pr);
            let change = (new_pr - old_pr).abs();
            if change > max_change {
                max_change = change;
            }

            new_pagerank.insert(node.clone(), new_pr);
        }

        pagerank = new_pagerank;

        // Check convergence
        if max_change < config.convergence_threshold {
            tracing::debug!(
                "PageRank converged after {} iterations (max_change: {:.6})",
                iteration + 1,
                max_change
            );
            break;
        }

        if iteration == config.max_iterations - 1 {
            tracing::warn!(
                "PageRank reached max iterations {} without full convergence (max_change: {:.6})",
                config.max_iterations,
                max_change
            );
        }
    }

    // Normalize scores to sum to 1.0 (optional, but good for verification)
    let total: f64 = pagerank.values().sum();
    if total > 0.0 {
        for pr in pagerank.values_mut() {
            *pr /= total;
        }
    }

    pagerank
}

/// Calculate PageRank for a single node
///
/// More efficient than computing for all nodes when only one score is needed.
/// Note: Still requires computing PageRank for the entire graph.
pub fn node_pagerank(
    adjacency: &GraphAdjacency,
    node: &GraphNodeId,
    config: &PageRankConfig,
) -> f64 {
    let all_scores = pagerank(adjacency, config);
    all_scores.get(node).copied().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_linear_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // A -> B -> C
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "B".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "C".to_string(), "LINK".to_string())],
        );

        graph
    }

    fn create_hub_graph() -> HashMap<(String, String), Vec<(String, String, String)>> {
        let mut graph = HashMap::new();

        // A -> Hub, B -> Hub, C -> Hub (Hub has high PageRank due to incoming links)
        graph.insert(
            ("ws".to_string(), "A".to_string()),
            vec![("ws".to_string(), "Hub".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "B".to_string()),
            vec![("ws".to_string(), "Hub".to_string(), "LINK".to_string())],
        );
        graph.insert(
            ("ws".to_string(), "C".to_string()),
            vec![("ws".to_string(), "Hub".to_string(), "LINK".to_string())],
        );

        graph
    }

    #[test]
    fn test_pagerank_linear_graph() {
        let graph = create_linear_graph();
        let config = PageRankConfig::default();
        let scores = pagerank(&graph, &config);

        // Should have scores for A, B, C
        assert_eq!(scores.len(), 3);

        // C should have highest PageRank (receives link from B, no outgoing links)
        let pr_a = scores
            .get(&("ws".to_string(), "A".to_string()))
            .copied()
            .unwrap_or(0.0);
        let pr_b = scores
            .get(&("ws".to_string(), "B".to_string()))
            .copied()
            .unwrap_or(0.0);
        let pr_c = scores
            .get(&("ws".to_string(), "C".to_string()))
            .copied()
            .unwrap_or(0.0);

        assert!(pr_c > pr_b, "C should have higher PR than B");
        assert!(pr_b > pr_a, "B should have higher PR than A");

        // Scores should sum to approximately 1.0
        let total: f64 = scores.values().sum();
        assert!((total - 1.0).abs() < 0.0001, "Scores should sum to 1.0");
    }

    #[test]
    fn test_pagerank_hub_graph() {
        let graph = create_hub_graph();
        let config = PageRankConfig::default();
        let scores = pagerank(&graph, &config);

        // Should have scores for A, B, C, Hub
        assert_eq!(scores.len(), 4);

        // Hub should have highest PageRank (receives links from A, B, C)
        let pr_hub = scores
            .get(&("ws".to_string(), "Hub".to_string()))
            .copied()
            .unwrap_or(0.0);
        let pr_a = scores
            .get(&("ws".to_string(), "A".to_string()))
            .copied()
            .unwrap_or(0.0);
        let pr_b = scores
            .get(&("ws".to_string(), "B".to_string()))
            .copied()
            .unwrap_or(0.0);
        let pr_c = scores
            .get(&("ws".to_string(), "C".to_string()))
            .copied()
            .unwrap_or(0.0);

        assert!(pr_hub > pr_a, "Hub should have higher PR than A");
        assert!(pr_hub > pr_b, "Hub should have higher PR than B");
        assert!(pr_hub > pr_c, "Hub should have higher PR than C");

        // A, B, C should have similar PageRank (dangling nodes)
        assert!((pr_a - pr_b).abs() < 0.01, "A and B should have similar PR");
        assert!((pr_b - pr_c).abs() < 0.01, "B and C should have similar PR");
    }

    #[test]
    fn test_pagerank_convergence() {
        let graph = create_hub_graph();
        let config = PageRankConfig {
            damping_factor: 0.85,
            max_iterations: 100,
            convergence_threshold: 0.0001,
        };

        let scores = pagerank(&graph, &config);

        // Should converge (test passes if it doesn't panic or timeout)
        assert!(scores.len() > 0);
    }

    #[test]
    fn test_node_pagerank() {
        let graph = create_hub_graph();
        let config = PageRankConfig::default();

        let hub_pr = node_pagerank(&graph, &("ws".to_string(), "Hub".to_string()), &config);

        assert!(hub_pr > 0.0, "Hub should have non-zero PageRank");
    }

    #[test]
    fn test_empty_graph() {
        let graph = HashMap::new();
        let config = PageRankConfig::default();
        let scores = pagerank(&graph, &config);

        assert_eq!(scores.len(), 0, "Empty graph should have no scores");
    }
}
