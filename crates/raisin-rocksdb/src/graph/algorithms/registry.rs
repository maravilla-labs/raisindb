//! Algorithm registry for dispatching computation based on config
//!
//! Provides a unified interface for executing any supported graph algorithm
//! based on the GraphAlgorithmConfig settings.

use super::{
    execute_betweenness_centrality, execute_bfs, execute_cdlp, execute_closeness_centrality,
    execute_connected_components, execute_lcc, execute_louvain, execute_pagerank, execute_sssp,
    execute_triangle_count,
};
use crate::graph::{CachedValue, GraphAlgorithm, GraphAlgorithmConfig, GraphCacheValue};
use raisin_error::{Error, Result};
use raisin_graph_algorithms::GraphProjection;
use std::collections::HashMap;

/// Result of an algorithm execution
#[derive(Debug)]
pub struct AlgorithmResult {
    /// Computed values per node
    pub values: HashMap<String, CachedValue>,
    /// Number of nodes processed
    pub node_count: usize,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
}

/// Executor for running graph algorithms
pub struct AlgorithmExecutor;

impl AlgorithmExecutor {
    /// Execute a graph algorithm based on configuration
    pub fn execute(
        config: &GraphAlgorithmConfig,
        projection: &mut GraphProjection,
    ) -> Result<AlgorithmResult> {
        let start = std::time::Instant::now();

        let values = match config.algorithm {
            GraphAlgorithm::PageRank => {
                let damping_factor = config.get_config_f64("damping_factor").unwrap_or(0.85);
                let max_iterations =
                    config.get_config_u64("max_iterations").unwrap_or(100) as usize;
                let tolerance = config
                    .get_config_f64("convergence_threshold")
                    .unwrap_or(1e-6);

                execute_pagerank(projection, damping_factor, max_iterations, tolerance)
            }
            GraphAlgorithm::Louvain => {
                let max_iterations = config.get_config_u64("max_iterations").unwrap_or(10) as usize;
                let resolution = config.get_config_f64("resolution").unwrap_or(1.0);

                execute_louvain(projection, max_iterations, resolution)
            }
            GraphAlgorithm::ConnectedComponents => execute_connected_components(projection),
            GraphAlgorithm::BetweennessCentrality => execute_betweenness_centrality(projection),
            GraphAlgorithm::ClosenessCentrality => execute_closeness_centrality(projection),
            GraphAlgorithm::TriangleCount => execute_triangle_count(projection),
            GraphAlgorithm::Bfs => {
                let source_node = config.get_config_str("source_node").ok_or_else(|| {
                    Error::Validation("BFS requires 'source_node' config parameter".to_string())
                })?;
                execute_bfs(projection, source_node)
            }
            GraphAlgorithm::Sssp => {
                let source_node = config.get_config_str("source_node").ok_or_else(|| {
                    Error::Validation("SSSP requires 'source_node' config parameter".to_string())
                })?;
                execute_sssp(projection, source_node)
            }
            GraphAlgorithm::Cdlp => {
                let max_iterations = config.get_config_u64("max_iterations").unwrap_or(10) as usize;
                execute_cdlp(projection, max_iterations)
            }
            GraphAlgorithm::Lcc => execute_lcc(projection),
            GraphAlgorithm::RelatesCache => {
                // RELATES cache is handled separately - it needs user context
                return Err(Error::Validation(
                    "RelatesCache should be handled by RelatesCacheExecutor".to_string(),
                ));
            }
        };

        let execution_time_ms = start.elapsed().as_millis() as u64;
        let node_count = values.len();

        Ok(AlgorithmResult {
            values,
            node_count,
            execution_time_ms,
        })
    }

    /// Build GraphCacheValue entries from algorithm results
    pub fn build_cache_entries(
        result: AlgorithmResult,
        source_revision: &str,
        config_revision: &str,
        ttl_seconds: u64,
    ) -> HashMap<String, GraphCacheValue> {
        super::build_cache_values(result.values, source_revision, config_revision, ttl_seconds)
    }
}

/// Registry of available algorithms with metadata
pub struct AlgorithmRegistry;

impl AlgorithmRegistry {
    /// Get the expected output type for an algorithm
    pub fn output_type(algorithm: &GraphAlgorithm) -> OutputType {
        match algorithm {
            GraphAlgorithm::PageRank => OutputType::Float,
            GraphAlgorithm::Louvain => OutputType::Integer,
            GraphAlgorithm::ConnectedComponents => OutputType::Integer,
            GraphAlgorithm::BetweennessCentrality => OutputType::Float,
            GraphAlgorithm::ClosenessCentrality => OutputType::Float,
            GraphAlgorithm::TriangleCount => OutputType::Integer,
            GraphAlgorithm::RelatesCache => OutputType::ReachabilitySet,
            GraphAlgorithm::Bfs => OutputType::Integer,
            GraphAlgorithm::Sssp => OutputType::Float,
            GraphAlgorithm::Cdlp => OutputType::Integer,
            GraphAlgorithm::Lcc => OutputType::Float,
        }
    }

    /// Get the default configuration for an algorithm
    pub fn default_config(algorithm: &GraphAlgorithm) -> HashMap<String, serde_json::Value> {
        let mut config = HashMap::new();

        match algorithm {
            GraphAlgorithm::PageRank => {
                config.insert("damping_factor".to_string(), serde_json::json!(0.85));
                config.insert(
                    "max_iterations".to_string(),
                    serde_json::Value::Number(100.into()),
                );
                config.insert("convergence_threshold".to_string(), serde_json::json!(1e-6));
            }
            GraphAlgorithm::Louvain => {
                config.insert("resolution".to_string(), serde_json::json!(1.0));
                config.insert(
                    "max_iterations".to_string(),
                    serde_json::Value::Number(10.into()),
                );
            }
            GraphAlgorithm::RelatesCache => {
                config.insert("max_depth".to_string(), serde_json::Value::Number(2.into()));
                config.insert(
                    "cache_scope".to_string(),
                    serde_json::Value::String("per_user".to_string()),
                );
            }
            GraphAlgorithm::Bfs => {
                config.insert(
                    "source_node".to_string(),
                    serde_json::Value::String(String::new()),
                );
            }
            GraphAlgorithm::Sssp => {
                config.insert(
                    "source_node".to_string(),
                    serde_json::Value::String(String::new()),
                );
            }
            GraphAlgorithm::Cdlp => {
                config.insert(
                    "max_iterations".to_string(),
                    serde_json::Value::Number(10.into()),
                );
            }
            _ => {
                // No default config needed
            }
        }

        config
    }

    /// Check if an algorithm requires special handling
    pub fn requires_special_handling(algorithm: &GraphAlgorithm) -> bool {
        matches!(algorithm, GraphAlgorithm::RelatesCache)
    }

    /// Get a human-readable description of the algorithm
    pub fn description(algorithm: &GraphAlgorithm) -> &'static str {
        match algorithm {
            GraphAlgorithm::PageRank => {
                "Computes PageRank centrality scores for all nodes in the graph"
            }
            GraphAlgorithm::Louvain => {
                "Detects communities using the Louvain modularity optimization algorithm"
            }
            GraphAlgorithm::ConnectedComponents => {
                "Identifies weakly connected components in the graph"
            }
            GraphAlgorithm::BetweennessCentrality => {
                "Computes betweenness centrality scores based on shortest paths"
            }
            GraphAlgorithm::ClosenessCentrality => {
                "Computes closeness centrality based on average shortest path distances"
            }
            GraphAlgorithm::TriangleCount => {
                "Counts the number of triangles each node participates in"
            }
            GraphAlgorithm::RelatesCache => {
                "Precomputes reachability sets for RELATES permission checks"
            }
            GraphAlgorithm::Bfs => {
                "Computes shortest distances from a source node using breadth-first search"
            }
            GraphAlgorithm::Sssp => {
                "Computes shortest weighted distances from a source node using Dijkstra's algorithm"
            }
            GraphAlgorithm::Cdlp => {
                "Detects communities using synchronous label propagation (LDBC Graphalytics)"
            }
            GraphAlgorithm::Lcc => "Computes local clustering coefficient for each node",
        }
    }
}

/// Output type for graph algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    /// Floating point value (PageRank, Betweenness)
    Float,
    /// Integer value (Louvain, Components, Triangles)
    Integer,
    /// Set of reachable nodes (RELATES cache)
    ReachabilitySet,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::*;

    fn make_test_config() -> GraphAlgorithmConfig {
        GraphAlgorithmConfig {
            id: "test-pagerank".to_string(),
            algorithm: GraphAlgorithm::PageRank,
            enabled: true,
            target: GraphTarget {
                mode: TargetMode::Branch,
                branches: vec!["main".to_string()],
                revisions: vec![],
                branch_pattern: None,
            },
            scope: GraphScope::default(),
            config: HashMap::new(),
            refresh: RefreshConfig::default(),
        }
    }

    fn create_test_graph() -> GraphProjection {
        let nodes = vec![
            "user1".to_string(),
            "user2".to_string(),
            "user3".to_string(),
        ];
        let edges = vec![
            ("user1".to_string(), "user2".to_string()),
            ("user2".to_string(), "user3".to_string()),
        ];
        GraphProjection::from_parts(nodes, edges)
    }

    #[test]
    fn test_executor_pagerank() {
        let config = make_test_config();
        let mut projection = create_test_graph();

        let result = AlgorithmExecutor::execute(&config, &mut projection).unwrap();

        assert_eq!(result.node_count, 3);
        assert!(result.execution_time_ms < 1000); // Should be fast

        for (_, value) in &result.values {
            assert!(value.as_float().is_some());
        }
    }

    #[test]
    fn test_registry_output_type() {
        assert_eq!(
            AlgorithmRegistry::output_type(&GraphAlgorithm::PageRank),
            OutputType::Float
        );
        assert_eq!(
            AlgorithmRegistry::output_type(&GraphAlgorithm::Louvain),
            OutputType::Integer
        );
        assert_eq!(
            AlgorithmRegistry::output_type(&GraphAlgorithm::RelatesCache),
            OutputType::ReachabilitySet
        );
    }

    #[test]
    fn test_registry_default_config() {
        let config = AlgorithmRegistry::default_config(&GraphAlgorithm::PageRank);
        assert!(config.contains_key("damping_factor"));
        assert!(config.contains_key("max_iterations"));
    }

    #[test]
    fn test_build_cache_entries() {
        let mut values = HashMap::new();
        values.insert("user1".to_string(), CachedValue::Float(0.85));

        let result = AlgorithmResult {
            values,
            node_count: 1,
            execution_time_ms: 10,
        };

        let entries = AlgorithmExecutor::build_cache_entries(result, "rev123", "config-v1", 300);

        assert_eq!(entries.len(), 1);
        let entry = entries.get("user1").unwrap();
        assert_eq!(entry.source_revision, "rev123");
    }
}
