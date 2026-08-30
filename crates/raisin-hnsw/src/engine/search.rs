// SPDX-License-Identifier: BSL-1.1

//! Search operations for the HNSW indexing engine.
//!
//! Provides nearest-neighbor search with workspace filtering, distance thresholds,
//! chunk-aware search modes, document deduplication, and position-based scoring.

use crate::index::{HnswIndex, ScopeFilterMode};
use crate::partition::PartitionId;
use crate::types::{
    deduplicate_by_document, ChunkSearchResult, DocumentSearchResult, ScoringConfig, SearchMode,
    SearchRequest, SearchResult, DEFAULT_MAX_DISTANCE, MAX_FETCH_K,
};
use raisin_error::Result;

use super::HnswIndexingEngine;

/// Chunk-collapse headroom.
///
/// The index holds one entry per CHUNK, so several entries can belong to the
/// same document and collapse into one row. Fetching exactly `k` would then
/// return fewer than `k` DOCUMENTS whenever any of them is chunked.
///
/// This is the ONLY over-draw the scoped paths need. The old workspace over-draw
/// (a flat 5x/10x on top of this) existed to compensate for post-filtering an
/// unfiltered walk; the walk is now filtered inside the index, so drawing more
/// candidates buys nothing but work. See `HnswIndex::search_scoped`.
const CHUNK_COLLAPSE_HEADROOM: usize = 2;

/// Fetch candidates for one workspace-scoped search, and account for a short
/// result IN ONE PLACE.
///
/// Both of this crate's search paths (`search_with_threshold` and
/// `search_chunks`) go through here. They previously carried mirrored copies of
/// the fetch sizing and the workspace post-filter, which is precisely how the
/// two drifted: one grew a distance-threshold override and a diagnostic dump,
/// the other did not.
fn fetch_scoped(
    index: &HnswIndex,
    query: &[f32],
    fetch_k: usize,
    workspaces: &[String],
    partition: &PartitionId,
) -> Result<Vec<SearchResult>> {
    let scoped = index.search_scoped(query, fetch_k, workspaces)?;

    if scoped.results.len() < fetch_k {
        match scoped.mode {
            // The misattribution this whole path exists to prevent. An operator
            // reading "returned 0 of 10" next to a workspace list concludes
            // "permissions dropped my rows"; on this branch the truth is that
            // the ANN walk never visited the workspace's region of the graph.
            ScopeFilterMode::PostFilter => tracing::warn!(
                filter_mode = "post_filter",
                partition = %partition,
                requested = fetch_k,
                returned = scoped.results.len(),
                drawn = scoped.drawn,
                in_scope_vectors = scoped.in_scope_total,
                workspaces = ?workspaces,
                "vector search came back SHORT after a post-filter: the graph walk was \
                 UNFILTERED and the workspace restriction was applied to its output, so this \
                 is INDEX SELECTIVITY (the walk never reached this scope), NOT a permission \
                 drop and NOT an empty index — the scope holds `in_scope_vectors` vectors"
            ),
            // Same shortfall, opposite meaning: the walk WAS scoped, so this is
            // the index's honest answer.
            ScopeFilterMode::IndexSide => tracing::debug!(
                filter_mode = "index_side",
                partition = %partition,
                requested = fetch_k,
                returned = scoped.results.len(),
                in_scope_vectors = scoped.in_scope_total,
                workspaces = ?workspaces,
                "vector search came back short from a workspace-filtered walk: the scope holds \
                 `in_scope_vectors` vectors in total, so this is the index's complete answer, \
                 not starvation"
            ),
            ScopeFilterMode::Unrestricted => tracing::debug!(
                filter_mode = "unrestricted",
                partition = %partition,
                requested = fetch_k,
                returned = scoped.results.len(),
                index_vectors = scoped.in_scope_total,
                "vector search came back short over the whole index"
            ),
        }
    }

    Ok(scoped.results)
}

impl HnswIndexingEngine {
    /// Search for nearest neighbors.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspaces` - Workspace filter. EMPTY means "every workspace"; a
    ///   non-empty slice restricts to exactly those.
    /// * `query` - Query vector
    /// * `k` - Number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results ordered by distance (closest first)
    ///
    /// # Workspace filtering happens INSIDE the graph walk
    ///
    /// usearch 2.24 takes a per-candidate predicate (`filtered_search`), so the
    /// workspace restriction is applied during the walk, not to its output. The
    /// walk keeps navigating through out-of-scope nodes and keeps going until it
    /// has `k` IN-SCOPE neighbours. A narrow scope inside a large index is
    /// therefore answered in full.
    ///
    /// It used to be a POST-filter over a fixed 10x over-draw, and a narrow
    /// scope came back short — often empty — because the unfiltered walk never
    /// visited that region of the graph. From the outside that was
    /// indistinguishable from permission filtering, which is what made it so
    /// expensive to diagnose. See `HnswIndex::search_scoped`, and
    /// `ScopeFilterMode` for how the one remaining fallback reports itself.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_threshold(
            tenant_id, repo_id, branch, partition, workspaces, query, k, None,
        )
    }

    /// Search for nearest neighbors with an optional distance threshold override.
    ///
    /// If `max_distance` is `None`, uses the default threshold (0.6 for cosine).
    /// Pass `Some(threshold)` to override per-query (e.g., from SQL WHERE clause
    /// or tenant configuration).
    #[allow(clippy::too_many_arguments)]
    pub fn search_with_threshold(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspaces: &[String],
        query: &[f32],
        k: usize,
        max_distance: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        let start = std::time::Instant::now();
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;

        let index = index_arc.read().unwrap();

        // Chunk-collapse headroom only — the workspace restriction is applied
        // inside the walk, so it needs no over-draw. An EMPTY `workspaces` slice
        // means "no filter"; it is never "match nothing" — a caller that
        // resolved to "nothing readable" must not reach this function at all.
        let fetch_k = k.saturating_mul(CHUNK_COLLAPSE_HEADROOM).min(MAX_FETCH_K);

        let mut results = fetch_scoped(&index, query, fetch_k, workspaces, partition)?;

        // Log all results before filtering for debugging.
        //
        // `chunk_id` as well as `node_id`: this line is the operator's view of
        // what the index actually holds, and after the chunk collapse below
        // several consecutive entries share one `node_id`. Printing only the
        // source id would make five distinct chunk vectors look like one entry
        // repeated five times.
        tracing::info!("Vector search raw results (before distance filtering):");
        for (i, result) in results.iter().enumerate() {
            tracing::info!(
                "  [{}] entry={} node={} workspace={} distance={:.4}",
                i + 1,
                result.chunk_id,
                result.node_id,
                result.workspace_id,
                result.distance
            );
        }

        // Filter by distance threshold to reject results that are too far away.
        // The cutoff is `crate::types::DEFAULT_MAX_DISTANCE` — one declaration,
        // shared with `search_chunks` below.
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

        // Collapse chunks to source documents, keeping each document's best
        // chunk, and limit to k DOCUMENTS.
        //
        // This is the whole reason the chunk split lives in the engine rather
        // than in each caller. The index files a chunked document under
        // `{node_id}#{chunk_index}`, an id that exists nowhere outside the
        // index: `storage.nodes().get(scope, "abc#3")` is `None`. Every caller
        // of this method fetches the node by `SearchResult::node_id`, so before
        // this every vector hit on a long document produced no row at all while
        // still consuming a result slot — the vector half of HYBRID_SEARCH was
        // silently dead for exactly the documents RAG exists to retrieve, and
        // an unchunked document's `abc` could never fuse with a chunked one's
        // `abc#0`. `node_id` is now the source id (see `SearchResult`), so the
        // fix reaches all four consumers without any of them knowing about
        // chunking.
        let results = deduplicate_by_document(results, k);

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
        partition: &PartitionId,
        request: &SearchRequest,
    ) -> Result<Vec<ChunkSearchResult>> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;
        let index = index_arc.read().unwrap();

        // Fetch size depends on the MODE only. The workspace restriction costs
        // nothing extra now that it is applied inside the walk; only the
        // document collapse needs headroom.
        let fetch_k = match request.mode {
            SearchMode::Chunks => request.k,
            SearchMode::Documents => request.k.saturating_mul(CHUNK_COLLAPSE_HEADROOM),
        }
        .min(MAX_FETCH_K);

        // The SAME scoped fetch as `search_with_threshold`, including its
        // short-result accounting. These two are this crate's mirrored search
        // paths; they share the one helper so they cannot drift again.
        let mut results = fetch_scoped(
            &index,
            &request.query_vector,
            fetch_k,
            &request.workspace_filters,
            partition,
        )?;

        // Apply distance threshold (use custom or the shared default)
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
        partition: &PartitionId,
        request: &SearchRequest,
    ) -> Result<Vec<DocumentSearchResult>> {
        // Force Documents mode
        let mut doc_request = request.clone();
        doc_request.mode = SearchMode::Documents;

        // Get chunk results
        let chunk_results =
            self.search_chunks(tenant_id, repo_id, branch, partition, &doc_request)?;

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
