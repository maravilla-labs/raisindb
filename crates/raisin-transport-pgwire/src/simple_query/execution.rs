// SPDX-License-Identifier: BSL-1.1

//! SQL query execution using QueryEngine.

use crate::auth::ApiKeyValidator;
use crate::result_encoder::{infer_schema_from_rows, ResultEncoder};
use futures::StreamExt;
use pgwire::api::results::{Response, Tag};
use pgwire::error::ErrorInfo;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::scope::RepoScope;
use raisin_storage::{Storage, WorkspaceRepository};
use std::sync::Arc;
use tracing::{debug, error, info};

use super::handler::RaisinSimpleQueryHandler;

impl<S, V, P> RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator,
    P: pgwire::api::auth::ServerParameterProvider,
{
    /// Execute a SQL query using QueryEngine and return results
    pub(super) async fn execute_query<'a>(
        &self,
        sql: &str,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        identity_auth: Option<AuthContext>,
    ) -> Response<'a> {
        info!(
            "Executing SQL query for tenant={}, repo={}, branch={}",
            tenant_id, repo_id, branch
        );
        debug!("SQL: {}", sql);

        // Fetch workspaces from storage to register them in the catalog
        let workspaces = match self
            .storage
            .workspaces()
            .list(RepoScope::new(tenant_id, repo_id))
            .await
        {
            Ok(ws) => ws,
            Err(e) => {
                error!("Failed to fetch workspaces: {}", e);
                return Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "XX000".to_string(),
                    format!("Failed to fetch workspaces: {}", e),
                )));
            }
        };

        debug!(
            "Found {} workspaces: {:?}",
            workspaces.len(),
            workspaces.iter().map(|w| &w.name).collect::<Vec<_>>()
        );

        // Create a catalog with all workspaces registered
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace in &workspaces {
            catalog.register_workspace(workspace.name.clone());
        }
        let catalog = Arc::new(catalog);

        // Create QueryEngine for this query with the catalog
        let mut engine = QueryEngine::new(self.storage.clone(), tenant_id, repo_id, branch)
            .with_catalog(catalog);

        // Set identity auth context if present (from SET app.user)
        if let Some(auth) = identity_auth {
            debug!("Using identity auth context: {}", auth.actor_id());
            engine = engine.with_auth(auth);
        }

        // Set indexing engines if available
        #[cfg(feature = "indexing")]
        {
            if let Some(ref indexing) = self.indexing_engine {
                engine = engine.with_indexing_engine(indexing.clone());
            }
            if let Some(ref hnsw) = self.hnsw_engine {
                engine = engine.with_hnsw_engine(hnsw.clone());
            }
        }

        // Wire schema stats cache for data-driven selectivity estimation
        if let Some(ref cache) = self.schema_stats_cache {
            engine = engine.with_schema_stats_cache(cache.clone());
        }

        // Execute the query using execute_batch() for proper scalar query support
        match engine.execute_batch(sql).await {
            Ok(mut row_stream) => {
                debug!("Query executed successfully, collecting results");

                let mut rows = Vec::new();
                while let Some(row_result) = row_stream.next().await {
                    match row_result {
                        Ok(row) => rows.push(row),
                        Err(e) => {
                            error!("Error reading row from stream: {}", e);
                            return Response::Error(Box::new(ErrorInfo::new(
                                "ERROR".to_string(),
                                "XX000".to_string(),
                                format!("Error reading query results: {}", e),
                            )));
                        }
                    }
                }

                debug!("Collected {} rows", rows.len());

                if rows.is_empty() {
                    debug!("No rows returned, sending empty result");
                    return Response::Execution(Tag::new("SELECT 0"));
                }

                let columns = infer_schema_from_rows(&rows);
                let schema = ResultEncoder::new().encode_schema(&columns);

                let response = ResultEncoder::build_query_response(rows, schema);

                Response::Query(response)
            }
            Err(e) => {
                error!("Query execution failed: {}", e);
                Response::Error(Box::new(ErrorInfo::new(
                    "ERROR".to_string(),
                    "42601".to_string(),
                    e.to_string(),
                )))
            }
        }
    }
}
