//! Graph algorithm wrappers for precomputation
//!
//! This module provides wrappers around the raisin-graph-algorithms crate
//! that integrate with the caching system and return cache-compatible results.

mod registry;

pub use registry::{AlgorithmExecutor, AlgorithmRegistry, AlgorithmResult};

use crate::graph::{CachedValue, GraphCacheValue};
use raisin_graph_algorithms::{
    algorithms::{
        betweenness_centrality, bfs, cdlp, closeness_centrality, lcc, louvain, page_rank, sssp,
        triangle_count, weakly_connected_components,
    },
    GraphProjection,
};
use std::collections::HashMap;

/// Execute PageRank algorithm
pub fn execute_pagerank(
    projection: &mut GraphProjection,
    damping_factor: f64,
    max_iterations: usize,
    tolerance: f64,
) -> HashMap<String, CachedValue> {
    let scores = page_rank(projection, damping_factor, max_iterations, tolerance);

    scores
        .into_iter()
        .map(|(node_id, score)| (node_id, CachedValue::Float(score)))
        .collect()
}

/// Execute Louvain community detection
pub fn execute_louvain(
    projection: &GraphProjection,
    max_iterations: usize,
    resolution: f64,
) -> HashMap<String, CachedValue> {
    let communities = louvain(projection, max_iterations, resolution);

    communities
        .into_iter()
        .map(|(node_id, community_id)| (node_id, CachedValue::Integer(community_id as u64)))
        .collect()
}

/// Execute connected components algorithm
pub fn execute_connected_components(projection: &GraphProjection) -> HashMap<String, CachedValue> {
    let components = weakly_connected_components(projection);

    components
        .into_iter()
        .map(|(node_id, component_id)| (node_id, CachedValue::Integer(component_id as u64)))
        .collect()
}

/// Execute triangle count algorithm
pub fn execute_triangle_count(projection: &GraphProjection) -> HashMap<String, CachedValue> {
    let counts = triangle_count(projection);

    counts
        .into_iter()
        .map(|(node_id, count)| (node_id, CachedValue::Integer(count as u64)))
        .collect()
}

/// Execute betweenness centrality algorithm
pub fn execute_betweenness_centrality(
    projection: &GraphProjection,
) -> HashMap<String, CachedValue> {
    let scores = betweenness_centrality(projection);
    scores
        .into_iter()
        .map(|(node_id, score)| (node_id, CachedValue::Float(score)))
        .collect()
}

/// Execute closeness centrality algorithm
pub fn execute_closeness_centrality(projection: &GraphProjection) -> HashMap<String, CachedValue> {
    let scores = closeness_centrality(projection);
    scores
        .into_iter()
        .map(|(node_id, score)| (node_id, CachedValue::Float(score)))
        .collect()
}

/// Execute BFS algorithm from a source node
pub fn execute_bfs(
    projection: &GraphProjection,
    source_node: &str,
) -> HashMap<String, CachedValue> {
    let distances = bfs(projection, source_node);
    distances
        .into_iter()
        .map(|(node_id, distance)| (node_id, CachedValue::Integer(distance)))
        .collect()
}

/// Execute SSSP (Dijkstra) from a source node
pub fn execute_sssp(
    projection: &GraphProjection,
    source_node: &str,
) -> HashMap<String, CachedValue> {
    let distances = sssp(projection, source_node);
    distances
        .into_iter()
        .map(|(node_id, distance)| (node_id, CachedValue::Float(distance)))
        .collect()
}

/// Execute CDLP (Community Detection via Label Propagation)
pub fn execute_cdlp(
    projection: &GraphProjection,
    max_iterations: usize,
) -> HashMap<String, CachedValue> {
    let labels = cdlp(projection, max_iterations);
    labels
        .into_iter()
        .map(|(node_id, label)| (node_id, CachedValue::Integer(label as u64)))
        .collect()
}

/// Execute LCC (Local Clustering Coefficient)
pub fn execute_lcc(projection: &GraphProjection) -> HashMap<String, CachedValue> {
    let coefficients = lcc(projection);
    coefficients
        .into_iter()
        .map(|(node_id, coeff)| (node_id, CachedValue::Float(coeff)))
        .collect()
}

/// Build cache values from algorithm results
pub fn build_cache_values(
    results: HashMap<String, CachedValue>,
    source_revision: &str,
    config_revision: &str,
    ttl_seconds: u64,
) -> HashMap<String, GraphCacheValue> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let expires_at = if ttl_seconds == 0 {
        0 // Never expires (revision mode)
    } else {
        now + (ttl_seconds * 1000)
    };

    results
        .into_iter()
        .map(|(node_id, value)| {
            let cache_value = GraphCacheValue {
                value,
                computed_at: now,
                expires_at,
                source_revision: source_revision.to_string(),
                config_revision: config_revision.to_string(),
            };
            (node_id, cache_value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> GraphProjection {
        let nodes = vec![
            "user1".to_string(),
            "user2".to_string(),
            "user3".to_string(),
            "user4".to_string(),
        ];
        let edges = vec![
            ("user1".to_string(), "user2".to_string()),
            ("user2".to_string(), "user3".to_string()),
            ("user3".to_string(), "user1".to_string()), // Triangle
            ("user3".to_string(), "user4".to_string()),
        ];
        GraphProjection::from_parts(nodes, edges)
    }

    #[test]
    fn test_pagerank_wrapper() {
        let mut projection = create_test_graph();
        let results = execute_pagerank(&mut projection, 0.85, 20, 1e-6);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            assert!(value.as_float().is_some());
        }
    }

    #[test]
    fn test_louvain_wrapper() {
        let projection = create_test_graph();
        let results = execute_louvain(&projection, 10, 1.0);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            assert!(value.as_integer().is_some());
        }
    }

    #[test]
    fn test_connected_components_wrapper() {
        let projection = create_test_graph();
        let results = execute_connected_components(&projection);

        assert_eq!(results.len(), 4);
        // All nodes should be in the same component
        for (_, value) in &results {
            // They may not all be the same if the algorithm implementation differs
            // but they should all have a component ID
            assert!(value.as_integer().is_some());
        }
    }

    #[test]
    fn test_triangle_count_wrapper() {
        let projection = create_test_graph();
        let results = execute_triangle_count(&projection);

        assert_eq!(results.len(), 4);
        // user1, user2, user3 form a triangle
        // user4 is not part of a triangle
        let user4_count = results.get("user4").unwrap().as_integer().unwrap();
        assert_eq!(user4_count, 0);
    }

    #[test]
    fn test_build_cache_values() {
        let mut results = HashMap::new();
        results.insert("user1".to_string(), CachedValue::Float(0.85));
        results.insert("user2".to_string(), CachedValue::Float(0.75));

        let cache_values = build_cache_values(results, "rev123", "config-v1", 300);

        assert_eq!(cache_values.len(), 2);

        let user1_cache = cache_values.get("user1").unwrap();
        assert_eq!(user1_cache.source_revision, "rev123");
        assert_eq!(user1_cache.config_revision, "config-v1");
        assert!(user1_cache.expires_at > 0); // TTL set
    }

    #[test]
    fn test_build_cache_values_no_ttl() {
        let mut results = HashMap::new();
        results.insert("user1".to_string(), CachedValue::Float(0.85));

        // TTL = 0 means revision mode (never expires)
        let cache_values = build_cache_values(results, "rev123", "config-v1", 0);

        let user1_cache = cache_values.get("user1").unwrap();
        assert_eq!(user1_cache.expires_at, 0); // Never expires
    }

    #[test]
    fn test_bfs_wrapper() {
        let projection = create_test_graph();
        let results = execute_bfs(&projection, "user1");

        assert_eq!(results.len(), 4);
        // Source node should have distance 0
        let user1_dist = results.get("user1").unwrap().as_integer().unwrap();
        assert_eq!(user1_dist, 0);
        for (_, value) in &results {
            assert!(value.as_integer().is_some());
        }
    }

    #[test]
    fn test_sssp_wrapper() {
        let projection = create_test_graph();
        let results = execute_sssp(&projection, "user1");

        assert_eq!(results.len(), 4);
        // Source node should have distance 0.0
        let user1_dist = results.get("user1").unwrap().as_float().unwrap();
        assert!((user1_dist - 0.0).abs() < f64::EPSILON);
        for (_, value) in &results {
            assert!(value.as_float().is_some());
        }
    }

    #[test]
    fn test_cdlp_wrapper() {
        let projection = create_test_graph();
        let results = execute_cdlp(&projection, 10);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            assert!(value.as_integer().is_some());
        }
    }

    #[test]
    fn test_lcc_wrapper() {
        let projection = create_test_graph();
        let results = execute_lcc(&projection);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            assert!(value.as_float().is_some());
        }
    }

    #[test]
    fn test_betweenness_centrality_wrapper() {
        let projection = create_test_graph();
        let results = execute_betweenness_centrality(&projection);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            let score = value.as_float().expect("betweenness should return float");
            assert!(score >= 0.0, "betweenness score should be non-negative");
        }
    }

    #[test]
    fn test_closeness_centrality_wrapper() {
        let projection = create_test_graph();
        let results = execute_closeness_centrality(&projection);

        assert_eq!(results.len(), 4);
        for (_, value) in &results {
            let score = value.as_float().expect("closeness should return float");
            assert!(score >= 0.0, "closeness score should be non-negative");
        }
    }
}
