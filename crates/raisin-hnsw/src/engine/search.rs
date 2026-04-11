// SPDX-License-Identifier: BSL-1.1

//! Search operations for the HNSW indexing engine.
//!
//! Provides nearest-neighbor search with workspace filtering, distance thresholds,
//! chunk-aware search modes, document deduplication, and position-based scoring.

use crate::types::{
    deduplicate_by_document, ChunkSearchResult, DocumentSearchResult, ScoringConfig, SearchMode,
    SearchRequest, SearchResult,
};
use raisin_error::Result;

use super::HnswIndexingEngine;

impl HnswIndexingEngine {
    /// Search for nearest neighbors.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Optional workspace filter (None = all workspaces)
    /// * `query` - Query vector
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results ordered by distance (closest first)
    ///
    /// # Workspace Filtering
    ///
    /// - `workspace_id = Some("ws1")` → Only returns results from workspace "ws1"
    /// - `workspace_id = None` → Returns results from ALL workspaces
    pub fn search(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: Option<&str>,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_threshold(tenant_id, repo_id, branch, workspace_id, query, k, None)
    }

    /// Search for nearest neighbors with an optional distance threshold override.
    ///
    /// If `max_distance` is `None`, uses the default threshold (0.6 for cosine).
    /// Pass `Some(threshold)` to override per-query (e.g., from SQL WHERE clause
    /// or tenant configuration).
    pub fn search_with_threshold(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: Option<&str>,
        query: &[f32],
        k: usize,
        max_distance: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        let start = std::time::Instant::now();
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch)?;

        let index = index_arc.read().unwrap();

        // Get more results than needed for filtering
        let fetch_k = if workspace_id.is_some() {
            k * 5 // Fetch 5x more to account for filtering
        } else {
            k
        };

        let mut results = index.search(query, fetch_k)?;

        // Filter by workspace if specified
        if let Some(ws_id) = workspace_id {
            results.retain(|r| r.workspace_id == ws_id);
        }

        // Log all results before filtering for debugging
        tracing::info!("Vector search raw results (before distance filtering):");
        for (i, result) in results.iter().enumerate() {
            tracing::info!(
                "  [{}] node={} workspace={} distance={:.4}",
                i + 1,
                result.node_id,
                result.workspace_id,
                result.distance
            );
        }

        // Filter by distance threshold to reject results that are too far away
        //
        // For cosine distance on normalized vectors:
        //   0.0 - 0.2  = Very similar   (cosine sim > 0.80)
        //   0.2 - 0.4  = Similar        (cosine sim 0.80-0.60)
        //   0.4 - 0.6  = Weakly related (cosine sim 0.60-0.40)
        //   0.6+       = Not related    (cosine sim < 0.40)
        const DEFAULT_MAX_DISTANCE: f32 = 0.6;
        let threshold = max_distance.unwrap_or(DEFAULT_MAX_DISTANCE);
        let before_filter_count = results.len();
        results.retain(|r| r.distance < threshold);
        let after_filter_count = results.len();

        if before_filter_count > after_filter_count {
            tracing::info!(
                "Filtered out {} results with distance >= {:.2}",
                before_filter_count - after_filter_count,
                threshold
            );
        }

        // Limit to k results
        results.truncate(k);

        // Record metrics
        self.metrics.record_search(start.elapsed(), results.len());

        tracing::info!(
            "Returning {} vector search results (after filtering and limit)",
            results.len()
        );

        Ok(results)
    }

    /// Search for nearest neighbors using a SearchRequest.
    ///
    /// This is the chunk-aware search API that supports both:
    /// - `SearchMode::Chunks`: Returns all matching chunks
    /// - `SearchMode::Documents`: Returns best chunk per document (deduplicated)
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `request` - Search request with mode and filters
    ///
    /// # Returns
    ///
    /// Vector of chunk search results ordered by distance or adjusted_score (closest first)
    pub fn search_chunks(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        request: &SearchRequest,
    ) -> Result<Vec<ChunkSearchResult>> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch)?;
        let index = index_arc.read().unwrap();

        // Determine fetch size based on mode and workspace filter
        let fetch_k = match request.mode {
            SearchMode::Chunks => {
                // For chunks mode, fetch more if we're filtering by workspace
                if request.workspace_filter.is_some() {
                    request.k * 5
                } else {
                    request.k
                }
            }
            SearchMode::Documents => {
                // For documents mode, we need to fetch more because:
                // 1. We might filter by workspace (5x)
                // 2. We need to deduplicate by document (2x more to ensure enough docs)
                if request.workspace_filter.is_some() {
                    request.k * 10
                } else {
                    request.k * 2
                }
            }
        };

        let mut results = index.search(&request.query_vector, fetch_k)?;

        // Filter by workspace if specified
        if let Some(ws_id) = &request.workspace_filter {
            results.retain(|r| &r.workspace_id == ws_id);
        }

        // Apply distance threshold (use custom or default MAX_DISTANCE)
        const DEFAULT_MAX_DISTANCE: f32 = 0.6;
        let max_distance = request.max_distance.unwrap_or(DEFAULT_MAX_DISTANCE);
        results.retain(|r| r.distance < max_distance);

        // Apply mode-specific logic
        let final_results = match request.mode {
            SearchMode::Chunks => {
                // Chunks mode: return raw results, limited to k
                results.truncate(request.k);
                results
            }
            SearchMode::Documents => {
                // Documents mode: deduplicate by source document
                deduplicate_by_document(results, request.k)
            }
        };

        // Convert to ChunkSearchResult
        let mut chunk_results: Vec<ChunkSearchResult> = final_results
            .into_iter()
            .map(|r| ChunkSearchResult::from_search_result(r, 1, None))
            .collect();

        // Apply scoring if configured
        if let Some(scoring_config) = &request.scoring {
            apply_scoring(&mut chunk_results, scoring_config);
        }

        Ok(chunk_results)
    }

    /// Search for nearest neighbors and return document results (deduplicated).
    ///
    /// This is a convenience method that always uses `SearchMode::Documents`.
    /// It returns one result per source document, choosing the best matching chunk.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `request` - Search request (mode will be overridden to Documents)
    ///
    /// # Returns
    ///
    /// Vector of document search results ordered by distance (closest first)
    pub fn search_documents(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        request: &SearchRequest,
    ) -> Result<Vec<DocumentSearchResult>> {
        // Force Documents mode
        let mut doc_request = request.clone();
        doc_request.mode = SearchMode::Documents;

        // Get chunk results
        let chunk_results = self.search_chunks(tenant_id, repo_id, branch, &doc_request)?;

        // Convert to DocumentSearchResult
        let doc_results = chunk_results
            .into_iter()
            .map(DocumentSearchResult::from_chunk_result)
            .collect();

        Ok(doc_results)
    }
}

/// Apply scoring configuration to chunk search results.
///
/// This function adjusts the similarity scores based on chunk position and other factors,
/// then re-sorts results by the adjusted score instead of raw distance.
///
/// # Arguments
///
/// * `results` - Mutable reference to chunk search results
/// * `config` - Scoring configuration
fn apply_scoring(results: &mut [ChunkSearchResult], config: &ScoringConfig) {
    for result in results.iter_mut() {
        // Start with base similarity score (convert distance to similarity)
        let mut score = result.similarity();

        // Apply position decay: earlier chunks score higher
        // position_factor decreases linearly with chunk_index
        let position_factor = 1.0 - (config.position_decay * result.chunk_index as f32);
        score *= position_factor.max(0.5); // Don't decay below 50%

        // Apply first chunk boost
        if result.chunk_index == 0 {
            score *= config.first_chunk_boost;
        }

        // Store adjusted score
        result.adjusted_score = Some(score);
    }

    // Re-sort by adjusted score (higher is better)
    results.sort_by(|a, b| {
        let score_a = a.adjusted_score.unwrap_or(a.similarity());
        let score_b = b.adjusted_score.unwrap_or(b.similarity());
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
