// SPDX-License-Identifier: BSL-1.1

//! Main hybrid search handler combining fulltext and vector search.

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};

use crate::state::AppState;

use super::rrf::merge_with_rrf;
use super::types::{HybridSearchQuery, HybridSearchResponse};

/// Perform hybrid search combining fulltext and vector search.
#[cfg(feature = "storage-rocksdb")]
pub async fn hybrid_search(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Query(params): Query<HybridSearchQuery>,
) -> impl IntoResponse {
    use super::fulltext::perform_fulltext_search;
    use super::vector::perform_vector_search;

    // Extract tenant from headers (for now, use "default")
    let tenant_id = "default";

    tracing::debug!(
        tenant = tenant_id,
        repo = %repo,
        strategy = %params.strategy,
        q = %params.q,
        has_vector = params.vector.is_some(),
        "Hybrid search request"
    );

    // Determine search strategy
    let use_fulltext = params.strategy == "fulltext" || params.strategy == "hybrid";
    let use_vector =
        (params.strategy == "vector" || params.strategy == "hybrid") && !params.q.is_empty();

    // Fulltext search results
    let fulltext_results = if use_fulltext && !params.q.is_empty() {
        match perform_fulltext_search(&state, tenant_id, &repo, &params) {
            Ok(results) => results,
            Err(e) => {
                tracing::error!("Fulltext search failed: {}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Vector search results (generate embedding from query text)
    let vector_results = if use_vector {
        match perform_vector_search(&state, tenant_id, &repo, &params).await {
            Ok(results) => results,
            Err(e) => {
                tracing::error!("Vector search failed: {}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Store counts before consuming the vectors
    let fulltext_count = fulltext_results.len();
    let vector_count = vector_results.len();

    // Merge results using RRF
    let merged_results = if use_fulltext && use_vector {
        merge_with_rrf(fulltext_results, vector_results, params.k, params.limit)
    } else if use_fulltext {
        fulltext_results.into_iter().take(params.limit).collect()
    } else if use_vector {
        vector_results.into_iter().take(params.limit).collect()
    } else {
        Vec::new()
    };

    Json(HybridSearchResponse {
        count: merged_results.len(),
        results: merged_results,
        strategy: params.strategy.clone(),
        fulltext_count,
        vector_count,
    })
}
