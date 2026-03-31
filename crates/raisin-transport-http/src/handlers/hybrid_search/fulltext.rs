// SPDX-License-Identifier: BSL-1.1

//! Fulltext search using Tantivy.

use raisin_hlc::HLC;

use crate::state::AppState;

use super::types::{HybridSearchQuery, HybridSearchResult};

/// Perform fulltext search using Tantivy.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn perform_fulltext_search(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    params: &HybridSearchQuery,
) -> Result<Vec<HybridSearchResult>, Box<dyn std::error::Error>> {
    use raisin_storage::{FullTextSearchQuery, IndexingEngine};

    let indexing_engine = state
        .indexing_engine
        .as_ref()
        .ok_or("Fulltext indexing not available")?;

    // Build search query
    let search_query = FullTextSearchQuery {
        tenant_id: tenant_id.to_string(),
        repo_id: repo.to_string(),
        workspace_ids: Some(vec![params.workspace.clone()]),
        branch: params.branch.clone(),
        language: "en".to_string(), // TODO: Get from request or config
        query: params.q.clone(),
        limit: params.limit * 2, // Get more results for RRF merging
        revision: None,          // HTTP API uses latest/HEAD by default
    };

    // Perform fulltext search
    let search_results = indexing_engine.search(&search_query)?;

    // Convert to HybridSearchResult format
    // Note: Tantivy doesn't index path, so we construct a placeholder
    let results: Vec<HybridSearchResult> = search_results
        .into_iter()
        .enumerate()
        .map(|(rank, result)| HybridSearchResult {
            node_id: result.node_id.clone(),
            name: result.name.unwrap_or_default(),
            node_type: result.node_type.unwrap_or_default(),
            path: format!("/{}", result.node_id), // TODO: Fetch from storage or add to Tantivy index
            workspace_id: result.workspace_id.clone(),
            score: 0.0, // Will be set by RRF
            fulltext_rank: Some(rank + 1),
            vector_distance: None,
            revision: result.revision.unwrap_or_else(|| HLC::new(0, 0)),
        })
        .collect();

    tracing::debug!(count = results.len(), "Fulltext search completed");

    Ok(results)
}
