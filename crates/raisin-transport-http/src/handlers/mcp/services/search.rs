// SPDX-License-Identifier: BSL-1.1

//! [`SearchProvider`] backed by the shared search implementation.
//!
//! # What this used to be
//!
//! A third implementation. It built its own `FullTextSearchQuery`, resolved its
//! own embedding provider, called the HNSW engine itself, and bound the caller's
//! identity as `_identity` -- so the `search_nodes` MCP tool returned node ids,
//! paths and types with NO row-level security, to whatever identity the session
//! had. Its full-text leg also hard-coded `language: "en"`.
//!
//! It now builds a `SearchArgs` and hands it to `QueryEngine::search`, exactly
//! as the SQL table function and the HTTP endpoint do: one scope resolver, one
//! per-hit `rls_filter_search_hit`, one over-fetch loop, one column set.

use std::future::Future;
use std::pin::Pin;

use raisin_mcp::{McpError, McpIdentity, SearchHit, SearchMode, SearchProvider, SearchQuery};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::physical_plan::search::args::{
    EmbeddingKindFilter, QueryInput, SearchArgs, SearchFunction,
};
use raisin_sql_execution::QueryEngine;

use crate::state::AppState;

/// Full-text + semantic search over RaisinDB content for the MCP `search_nodes`
/// tool.
#[derive(Clone)]
pub(in crate::handlers::mcp) struct HttpSearchProvider {
    state: AppState,
    tenant_id: String,
    repo: String,
    /// The request's resolved auth context -- the SAME one the data tools get.
    ///
    /// Passing it here rather than rebuilding it from the `McpIdentity` is
    /// load-bearing: a rebuilt context carries no resolved permissions, which
    /// RLS treats as deny-all. `None` is the system/internal caller.
    auth: Option<AuthContext>,
}

impl HttpSearchProvider {
    /// Bind a search provider to the request's tenant / repo scope and identity.
    pub(in crate::handlers::mcp) fn new(
        state: AppState,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        auth: Option<AuthContext>,
    ) -> Self {
        Self {
            state,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            auth,
        }
    }

    async fn run(&self, query: SearchQuery) -> raisin_mcp::Result<Vec<SearchHit>> {
        // A mode picks a leg by zeroing the other weight, which SKIPS it
        // entirely -- so `fulltext` mode needs no embedding provider and
        // `vector` mode needs no full-text index.
        let (fulltext_weight, vector_weight) = match query.mode {
            SearchMode::Fulltext => (1.0, 0.0),
            SearchMode::Vector => (0.0, 1.0),
        };

        let granularity = match query.granularity.as_deref() {
            Some(raw) => raisin_sql_execution::physical_plan::search::args::Granularity::parse(
                raw,
                "search_nodes",
            )
            .map_err(|e| McpError::protocol(e.to_string()))?,
            None => raisin_sql_execution::physical_plan::search::args::Granularity::default(),
        };

        let args = SearchArgs::from_api(
            SearchFunction::Hybrid,
            QueryInput::Text(query.query.clone()),
            query.limit.max(1),
            &query.workspace,
            None,
            "en",
            fulltext_weight,
            vector_weight,
            None,
            // Text only. The MCP tool returns node hits to an agent that asked
            // to search documents; folding an image tower in silently would
            // change what every existing agent's `search_nodes` returns.
            EmbeddingKindFilter::default(),
            granularity,
        )
        .map_err(|e| McpError::protocol(e.to_string()))?;

        let mut engine = QueryEngine::new(
            self.state.storage.clone(),
            self.tenant_id.as_str(),
            self.repo.as_str(),
            query.branch.as_str(),
        );
        if let Some(ref indexing) = self.state.indexing_engine {
            engine = engine.with_indexing_engine(indexing.clone());
        }
        if let Some(ref hnsw) = self.state.hnsw_engine {
            engine = engine.with_hnsw_engine(hnsw.clone());
        }
        if let Some(ref auth) = self.auth {
            engine = engine.with_auth(auth.clone());
        }

        // `node_type` is NOT pushed down as a filter argument: the index field
        // it would reach is multi-valued (node_type + archetype + nested
        // element_type) and so over-matches, and there is no residual filter on
        // this path to make that safe. Filtering the returned rows is exact.
        let wanted_type = query.node_type.clone();

        let rows = engine
            .search(args, "search_nodes")
            .await
            .map_err(|e| McpError::protocol(format!("search failed: {e}")))?;

        Ok(rows
            .iter()
            .filter(|row| match &wanted_type {
                Some(t) => text(row, "node_type").as_deref() == Some(t.as_str()),
                None => true,
            })
            .map(|row| SearchHit {
                node_id: text(row, "node_id").unwrap_or_default(),
                workspace: text(row, "workspace_id").unwrap_or_default(),
                score: number(row, "score").unwrap_or(0.0) as f32,
                path: text(row, "path"),
                node_type: text(row, "node_type"),
                // Already produced by the shared emit loop; this surface used
                // to discard all four, which made `search_nodes` unable to
                // answer the one question a RAG agent asks — what did the
                // matching passage SAY?
                chunk_index: number(row, "chunk_index").map(|v| v as i64),
                chunk_text: text(row, "chunk_text"),
                chunk_text_source: text(row, "chunk_text_source"),
                vector_distance: number(row, "vector_distance").map(|v| v as f32),
            })
            .collect())
    }
}

/// Rows arrive qualified (`search_nodes.path`), so match on the suffix.
fn column<'a>(row: &'a raisin_sql_execution::Row, name: &str) -> Option<&'a PropertyValue> {
    let suffix = format!(".{name}");
    row.columns
        .iter()
        .find(|(key, _)| key.as_str() == name || key.ends_with(&suffix))
        .map(|(_, value)| value)
}

fn text(row: &raisin_sql_execution::Row, name: &str) -> Option<String> {
    match column(row, name) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn number(row: &raisin_sql_execution::Row, name: &str) -> Option<f64> {
    match column(row, name) {
        Some(PropertyValue::Float(v)) => Some(*v),
        Some(PropertyValue::Integer(v)) => Some(*v as f64),
        _ => None,
    }
}

impl SearchProvider for HttpSearchProvider {
    fn search<'a>(
        &'a self,
        _identity: &'a McpIdentity,
        query: SearchQuery,
    ) -> Pin<Box<dyn Future<Output = raisin_mcp::Result<Vec<SearchHit>>> + Send + 'a>> {
        // The identity is deliberately unused HERE: enforcement uses the
        // resolved `AuthContext` captured at construction, because an identity
        // re-derived at this point carries no permissions and RLS would deny
        // everything. See the `auth` field.
        Box::pin(async move { self.run(query).await })
    }
}
