// SPDX-License-Identifier: BSL-1.1

//! Hybrid search combining fulltext and vector similarity search.
//!
//! This module implements Reciprocal Rank Fusion (RRF) to merge results from
//! multiple search strategies:
//! - Fulltext search (Tantivy)
//! - Vector similarity search (HNSW)
//!
//! # RRF Algorithm
//!
//! For each document d and rank lists R1, R2, ..., Rn:
//! ```text
//! RRF(d) = sum of 1 / (k + rank_i(d))
//! ```
//! where k is a constant (typically 60) and rank_i(d) is the rank of document d in list i.

#[cfg(feature = "storage-rocksdb")]
mod fulltext;
mod handler;
mod rrf;
mod types;
#[cfg(feature = "storage-rocksdb")]
mod vector;

// Re-export public API
#[cfg(feature = "storage-rocksdb")]
pub use handler::hybrid_search;
pub use types::{HybridSearchQuery, HybridSearchResponse, HybridSearchResult};

#[cfg(test)]
mod tests {
    use super::rrf::{merge_with_rrf, parse_vector};
    use super::types::HybridSearchResult;
    use raisin_hlc::HLC;

    #[test]
    fn test_parse_vector_comma_separated() {
        let vec = parse_vector("0.1,0.2,0.3").unwrap();
        assert_eq!(vec, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_parse_vector_json() {
        let vec = parse_vector("[0.1,0.2,0.3]").unwrap();
        assert_eq!(vec, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_rrf_merge() {
        let fulltext = vec![
            HybridSearchResult {
                node_id: "A".to_string(),
                name: "Node A".to_string(),
                node_type: "Page".to_string(),
                path: "/A".to_string(),
                workspace_id: "default".to_string(),
                score: 0.0,
                fulltext_rank: Some(1),
                vector_distance: None,
                revision: HLC::new(1, 0),
            },
            HybridSearchResult {
                node_id: "B".to_string(),
                name: "Node B".to_string(),
                node_type: "Page".to_string(),
                path: "/B".to_string(),
                workspace_id: "default".to_string(),
                score: 0.0,
                fulltext_rank: Some(2),
                vector_distance: None,
                revision: HLC::new(1, 0),
            },
        ];

        let vector = vec![
            HybridSearchResult {
                node_id: "B".to_string(),
                name: "Node B".to_string(),
                node_type: "Page".to_string(),
                path: "/B".to_string(),
                workspace_id: "default".to_string(),
                score: 0.0,
                fulltext_rank: None,
                vector_distance: Some(0.5),
                revision: HLC::new(1, 0),
            },
            HybridSearchResult {
                node_id: "C".to_string(),
                name: "Node C".to_string(),
                node_type: "Page".to_string(),
                path: "/C".to_string(),
                workspace_id: "default".to_string(),
                score: 0.0,
                fulltext_rank: None,
                vector_distance: Some(0.8),
                revision: HLC::new(1, 0),
            },
        ];

        let merged = merge_with_rrf(fulltext, vector, 60.0, 10);

        // B should be first (appears in both lists)
        assert_eq!(merged[0].node_id, "B");
        assert!(merged[0].score > merged[1].score);

        // B should have both fulltext rank and vector distance
        assert!(merged[0].fulltext_rank.is_some());
        assert!(merged[0].vector_distance.is_some());
    }
}
