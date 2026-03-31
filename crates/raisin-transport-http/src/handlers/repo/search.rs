// SPDX-License-Identifier: BSL-1.1

//! Full-text search handler for repository content.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
#[cfg(feature = "storage-rocksdb")]
use raisin_storage::{FullTextSearchQuery, IndexingEngine, StorageScope};
use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage};

use crate::{error::ApiError, state::AppState};

#[derive(Debug, serde::Deserialize)]
pub struct FullTextSearchRequest {
    /// Search query string (supports Tantivy query syntax)
    pub query: String,
    /// Optional workspace filter
    pub workspace: Option<String>,
    /// Language code for search (defaults to repository default_language)
    pub language: Option<String>,
    /// Maximum number of results (default: 20, max: 100)
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Serialize)]
pub struct SearchResultItem {
    /// Node ID
    pub node_id: String,
    /// Workspace ID
    pub workspace_id: String,
    /// Node name
    pub name: String,
    /// Node path
    pub path: String,
    /// Node type
    pub node_type: String,
    /// Relevance score
    pub score: f32,
}

/// Full-text search across repository
///
/// POST /api/repository/{repo}/{branch}/fulltext/search
///
/// Request body:
/// ```json
/// {
///   "query": "search terms",
///   "workspace": "optional-workspace-filter",
///   "language": "en",
///   "limit": 20
/// }
/// ```
#[cfg(feature = "storage-rocksdb")]
pub async fn fulltext_search(
    State(state): State<AppState>,
    Path((repo, branch)): Path<(String, String)>,
    Json(req): Json<FullTextSearchRequest>,
) -> Result<Json<Vec<SearchResultItem>>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Check if indexing engine is available
    let engine = state.indexing_engine.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "INDEXING_DISABLED",
            "Full-text search is not available".to_string(),
        )
    })?;

    // Get repository info to determine default language
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, &repo)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Repository {} not found", repo)))?;

    // Use provided language or fall back to repository default
    let language = req
        .language
        .unwrap_or_else(|| repo_info.config.default_language.clone());

    // Validate limit
    let limit = req.limit.unwrap_or(20).min(100);

    // If workspace filter is provided, search only that workspace
    if let Some(workspace_id) = req.workspace {
        let search_query = FullTextSearchQuery {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.clone(),
            workspace_ids: Some(vec![workspace_id]),
            branch: branch.clone(),
            language,
            query: req.query,
            limit,
            revision: None, // HTTP API uses latest/HEAD by default
        };

        // Execute search
        let results = engine.search(&search_query).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "SEARCH_ERROR",
                format!("Search failed: {}", e),
            )
        })?;

        // Fetch node details for each result
        let mut items = Vec::new();
        for result in results {
            if let Ok(Some(node)) = state
                .storage()
                .nodes()
                .get(
                    StorageScope::new(tenant_id, &repo, &branch, &result.workspace_id),
                    &result.node_id,
                    None,
                )
                .await
            {
                items.push(SearchResultItem {
                    node_id: result.node_id,
                    workspace_id: result.workspace_id.clone(),
                    name: node.name,
                    path: node.path,
                    node_type: node.node_type,
                    score: result.score,
                });
            }
        }

        return Ok(Json(items));
    }

    // No workspace filter: search across all workspaces (optimized single query)
    let search_query = FullTextSearchQuery {
        tenant_id: tenant_id.to_string(),
        repo_id: repo.clone(),
        workspace_ids: None, // Cross-workspace search
        branch: branch.clone(),
        language: language.clone(),
        query: req.query.clone(),
        limit,
        revision: None, // HTTP API uses latest/HEAD by default
    };

    // Execute cross-workspace search
    let results = engine.search(&search_query).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SEARCH_ERROR",
            format!("Fulltext search failed: {}", e),
        )
    })?;

    // Fetch node details for each result (workspace comes from search result)
    let mut all_items = Vec::new();
    for result in results {
        if let Ok(Some(node)) = state
            .storage()
            .nodes()
            .get(
                StorageScope::new(tenant_id, &repo, &branch, &result.workspace_id),
                &result.node_id,
                None,
            )
            .await
        {
            all_items.push(SearchResultItem {
                node_id: result.node_id,
                workspace_id: result.workspace_id.clone(),
                name: node.name,
                path: node.path,
                node_type: node.node_type,
                score: result.score,
            });
        }
    }

    // Results are already sorted by score from Tantivy, but limit to requested amount
    all_items.truncate(limit);

    Ok(Json(all_items))
}
