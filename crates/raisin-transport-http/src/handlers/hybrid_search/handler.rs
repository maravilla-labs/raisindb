// SPDX-License-Identifier: BSL-1.1

//! The HTTP hybrid-search endpoint, on top of the shared search implementation.
//!
//! # What this used to be
//!
//! A second complete implementation: its own full-text leg, its own vector leg,
//! and its own Reciprocal Rank Fusion in `rrf.rs` that keyed
//! `HashMap<String, _>` on `node_id` ALONE -- the exact bug the SQL side had
//! already fixed by keying on `(workspace_id, node_id)`, so two different nodes
//! with the same id in two workspaces collapsed into one row with the wrong
//! metadata. The module contained no reference to `auth` anywhere, which meant
//! the endpoint returned node names, paths and types with NO row-level security
//! at all.
//!
//! It now builds a `SearchArgs` and hands it to
//! `QueryEngine::search`, the same code path `SELECT ... FROM HYBRID_SEARCH(...)`
//! takes: same scope resolution, same per-hit `rls_filter_search_hit`, same
//! over-fetch/backfill so `limit` means rows delivered, same fusion.

use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;

use crate::{middleware::TenantInfo, state::AppState};

use super::types::{HybridSearchQuery, HybridSearchResponse, HybridSearchResult};

/// Perform hybrid search combining fulltext and vector search.
#[cfg(feature = "storage-rocksdb")]
pub async fn hybrid_search(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Path(repo): Path<String>,
    Query(params): Query<HybridSearchQuery>,
) -> impl IntoResponse {
    use raisin_sql_execution::physical_plan::search::args::{
        EmbeddingKindFilter, QueryInput, SearchArgs, SearchFunction,
    };
    use raisin_sql_execution::QueryEngine;

    let tenant_id = tenant_info.tenant_id.as_str();

    // `strategy` selects the legs by setting a weight to zero, which is exactly
    // what the SQL surface does -- a zero weight SKIPS the leg, including the
    // embedding round trip, so `strategy=fulltext` works on a tenant with no
    // embedding provider configured.
    let (fulltext_weight, vector_weight) = match params.strategy.as_str() {
        "fulltext" => (1.0, 0.0),
        "vector" => (0.0, 1.0),
        _ => (1.0, 1.0),
    };

    if params.k != 60.0 {
        // Accepted and ignored, documented deprecated. RRF's k is a GLOBAL
        // constant: varying it per request makes two callers' scores
        // incomparable, and it invites tuning that hides a dead leg (lowering k
        // "improves" results when the vector half is not running at all).
        tracing::warn!(
            k = params.k,
            "hybrid search: the `k` parameter is deprecated and ignored; RRF k is \
             fixed at 60"
        );
    }

    let granularity = match raisin_sql_execution::physical_plan::search::args::Granularity::parse(
        &params.granularity,
        "hybrid search",
    ) {
        Ok(g) => g,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let kind = match EmbeddingKindFilter::parse(&params.kind, "hybrid search") {
        Ok(kind) => kind,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let args = match SearchArgs::from_api(
        SearchFunction::Hybrid,
        QueryInput::Text(params.q.clone()),
        params.limit,
        &params.workspace,
        None,
        "en",
        fulltext_weight,
        vector_weight,
        None,
        kind,
        granularity,
    ) {
        Ok(args) => args,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let mut engine = QueryEngine::new(
        state.storage.clone(),
        tenant_id,
        repo.as_str(),
        params.branch.as_str(),
    );
    if let Some(ref indexing) = state.indexing_engine {
        engine = engine.with_indexing_engine(indexing.clone());
    }
    if let Some(ref hnsw) = state.hnsw_engine {
        engine = engine.with_hnsw_engine(hnsw.clone());
    }
    // Row-level security. Without this the endpoint answers with node names,
    // paths and types the caller may have no permission to read -- which is
    // precisely what it did before.
    if let Some(Extension(auth)) = auth {
        engine = engine.with_auth(auth);
    }

    let rows = match engine.search(args, "hybrid_search").await {
        Ok(rows) => rows,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let results: Vec<HybridSearchResult> = rows.iter().map(row_to_result).collect();
    let fulltext_count = results.iter().filter(|r| r.fulltext_rank.is_some()).count();
    let vector_count = results
        .iter()
        .filter(|r| r.vector_distance.is_some())
        .count();

    Json(HybridSearchResponse {
        count: results.len(),
        results,
        strategy: params.strategy.clone(),
        fulltext_count,
        vector_count,
    })
    .into_response()
}

fn row_to_result(row: &raisin_sql_execution::Row) -> HybridSearchResult {
    HybridSearchResult {
        node_id: text(row, "node_id"),
        name: text(row, "name"),
        node_type: text(row, "node_type"),
        path: text(row, "path"),
        workspace_id: text(row, "workspace_id"),
        score: number(row, "score").unwrap_or(0.0) as f32,
        fulltext_rank: number(row, "fulltext_rank").map(|v| v as usize),
        vector_distance: number(row, "vector_distance").map(|v| v as f32),
        revision: number(row, "revision").unwrap_or(0.0) as i64,
        chunk_index: number(row, "chunk_index").map(|v| v as i64),
        // The passage, and how confident we are that it IS the passage. Both
        // were already produced by the shared emit loop and discarded here.
        chunk_text: opt_text(row, "chunk_text"),
        chunk_text_source: text(row, "chunk_text_source"),
    }
}

fn opt_text(row: &raisin_sql_execution::Row, name: &str) -> Option<String> {
    match column(row, name) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Rows arrive qualified (`hybrid_search.path`), so match on the suffix.
fn column<'a>(row: &'a raisin_sql_execution::Row, name: &str) -> Option<&'a PropertyValue> {
    let suffix = format!(".{name}");
    row.columns
        .iter()
        .find(|(key, _)| key.as_str() == name || key.ends_with(&suffix))
        .map(|(_, value)| value)
}

fn text(row: &raisin_sql_execution::Row, name: &str) -> String {
    match column(row, name) {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn number(row: &raisin_sql_execution::Row, name: &str) -> Option<f64> {
    match column(row, name) {
        Some(PropertyValue::Float(v)) => Some(*v),
        Some(PropertyValue::Integer(v)) => Some(*v as f64),
        _ => None,
    }
}
