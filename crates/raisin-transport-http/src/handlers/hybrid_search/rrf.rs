// SPDX-License-Identifier: BSL-1.1

//! Reciprocal Rank Fusion (RRF) merge algorithm and vector parsing utilities.
//!
//! # RRF Algorithm
//!
//! For each document d and rank lists R1, R2, ..., Rn:
//! ```text
//! RRF(d) = sum of 1 / (k + rank_i(d))
//! ```
//! where k is a constant (typically 60) and rank_i(d) is the rank of document d in list i.

use raisin_hlc::HLC;
use std::collections::HashMap;

use super::types::HybridSearchResult;

/// Parse vector from query parameter.
///
/// Supports:
/// - Comma-separated floats: "0.1,0.2,0.3"
/// - JSON array: "[0.1,0.2,0.3]"
pub(super) fn parse_vector(input: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let input = input.trim();

    // Try JSON array first
    if input.starts_with('[') {
        let vec: Vec<f32> = serde_json::from_str(input)?;
        return Ok(vec);
    }

    // Try comma-separated
    let vec: Result<Vec<f32>, _> = input.split(',').map(|s| s.trim().parse()).collect();

    Ok(vec?)
}

/// Merge results using Reciprocal Rank Fusion (RRF).
///
/// # Algorithm
///
/// For each document d:
/// ```text
/// RRF(d) = sum of 1 / (k + rank_i(d))
/// ```
///
/// Where:
/// - k = constant (typically 60)
/// - rank_i(d) = rank of document d in list i (1-indexed)
///
/// # Example
///
/// Given two ranked lists:
/// - List 1: [A, B, C]
/// - List 2: [B, A, D]
///
/// RRF scores (k=60):
/// - A: 1/(60+1) + 1/(60+2) = 0.0325
/// - B: 1/(60+2) + 1/(60+1) = 0.0325
/// - C: 1/(60+3) = 0.0159
/// - D: 1/(60+3) = 0.0159
pub(super) fn merge_with_rrf(
    fulltext_results: Vec<HybridSearchResult>,
    vector_results: Vec<HybridSearchResult>,
    k: f32,
    limit: usize,
) -> Vec<HybridSearchResult> {
    /// Accumulated RRF score entry: (score, fulltext_rank, vector_distance, revision, name, node_type, path, workspace_id)
    type RrfScoreEntry = (
        f32,
        Option<usize>,
        Option<f32>,
        HLC,
        String,
        String,
        String,
        String,
    );

    let mut score_map: HashMap<String, RrfScoreEntry> = HashMap::new();

    // Add fulltext results
    for (rank, result) in fulltext_results.into_iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank + 1) as f32);
        score_map.insert(
            result.node_id.clone(),
            (
                rrf_score,
                Some(rank + 1),
                None,
                result.revision,
                result.name,
                result.node_type,
                result.path,
                result.workspace_id,
            ),
        );
    }

    // Add vector results
    for (rank, result) in vector_results.into_iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank + 1) as f32);

        score_map
            .entry(result.node_id.clone())
            .and_modify(|entry| {
                entry.0 += rrf_score; // Add to existing score
                entry.2 = result.vector_distance; // Set vector distance
            })
            .or_insert((
                rrf_score,
                None,
                result.vector_distance,
                result.revision,
                result.name,
                result.node_type,
                result.path,
                result.workspace_id,
            ));
    }

    // Sort by RRF score (descending)
    let mut merged: Vec<HybridSearchResult> = score_map
        .into_iter()
        .map(
            |(
                node_id,
                (
                    score,
                    fulltext_rank,
                    vector_distance,
                    revision,
                    name,
                    node_type,
                    path,
                    workspace_id,
                ),
            )| {
                HybridSearchResult {
                    node_id,
                    name,
                    node_type,
                    path,
                    workspace_id,
                    score,
                    fulltext_rank,
                    vector_distance,
                    revision,
                }
            },
        )
        .collect();

    merged.sort_by(|a, b| b.score.total_cmp(&a.score));

    merged.into_iter().take(limit).collect()
}
