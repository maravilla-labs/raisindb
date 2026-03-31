// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Implementation of pgwire's [`ExtendedQueryHandler`] trait.
//!
//! This is the main dispatch point for the extended query protocol:
//! `do_query` routes SQL to the appropriate session handler or the
//! query engine, while `do_describe_statement` / `do_describe_portal`
//! provide schema metadata to clients.

use crate::auth::ApiKeyValidator;
use crate::result_encoder::{infer_schema_from_rows, ResultEncoder};
use async_trait::async_trait;
use futures::StreamExt;
use pgwire::api::portal::Portal;
use pgwire::api::query::ExtendedQueryHandler;
use pgwire::api::results::{
    DescribePortalResponse, DescribeStatementResponse, FieldInfo, Response, Tag,
};
use pgwire::api::stmt::StoredStatement;
use pgwire::api::{ClientInfo, Type};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::scope::RepoScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{RepositoryManagementRepository, Storage, WorkspaceRepository};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::statement::{RaisinQueryParser, RaisinStatement};
use super::RaisinExtendedQueryHandler;

#[async_trait]
impl<S, V, P> ExtendedQueryHandler for RaisinExtendedQueryHandler<S, V, P>
where
    S: Storage + TransactionalStorage + 'static,
    V: ApiKeyValidator + 'static,
    P: pgwire::api::auth::ServerParameterProvider + 'static,
{
    type Statement = RaisinStatement;
    type QueryParser = RaisinQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        self.query_parser.clone()
    }

    async fn do_query<'a, C>(
        &self,
        client: &mut C,
        portal: &'a Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response<'a>>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        debug!("Executing extended query");

        // Get connection context from auth handler (same pattern as simple_query.rs)
        let context = self.auth_handler.get_context(client).ok_or_else(|| {
            error!("No connection context found - client not authenticated");
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "28000".to_owned(), // Invalid authorization
                "No connection context found - client not authenticated".to_owned(),
            )))
        })?;

        info!(
            "Executing extended query for tenant={}, repo={}",
            context.tenant_id, context.repository
        );

        // Bind parameters to SQL
        let sql = self.bind_parameters(portal).map_err(|e| {
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "42P02".to_owned(), // Undefined parameter
                e.to_string(),
            )))
        })?;

        debug!("SQL after parameter binding: {}", sql);

        // Handle system queries (SET, SHOW, RESET) - same pattern as simple_query.rs
        let sql_lower = sql.trim().to_lowercase();

        // Handle SET app.user = '<jwt>' - set identity context (BEFORE generic SET handling)
        if sql_lower.starts_with("set local app.user") || sql_lower.starts_with("set app.user") {
            debug!("Extended query: Handling SET app.user command: {}", sql);
            return Ok(self.handle_set_identity_user(&sql, client, &context).await);
        }

        // Handle RESET app.user - clear identity context
        if sql_lower.starts_with("reset app.user") {
            debug!("Extended query: Handling RESET app.user command");
            self.auth_handler.clear_identity_auth(client);
            return Ok(Response::Execution(Tag::new("RESET")));
        }

        // Handle SET app.branch = 'x' / SET LOCAL app.branch = 'x' (BEFORE generic SET handling)
        if sql_lower.starts_with("set local app.branch") || sql_lower.starts_with("set app.branch")
        {
            debug!("Extended query: Handling SET app.branch command: {}", sql);
            return Ok(self.handle_set_branch(&sql, client, &sql_lower));
        }

        // Handle USE BRANCH 'x' / USE LOCAL BRANCH 'x'
        if sql_lower.starts_with("use branch") || sql_lower.starts_with("use local branch") {
            debug!("Extended query: Handling USE BRANCH command: {}", sql);
            return Ok(self.handle_use_branch(&sql, client, &sql_lower));
        }

        // Handle SHOW app.branch / SHOW CURRENT BRANCH
        if sql_lower.starts_with("show app.branch") || sql_lower.starts_with("show current branch")
        {
            debug!("Extended query: Handling SHOW branch command");
            return Ok(self.handle_show_branch(client, &context).await);
        }

        // Handle other SET commands - acknowledge but don't execute
        if sql_lower.starts_with("set ") {
            debug!("Extended query: Acknowledging SET command: {}", sql);
            return Ok(Response::Execution(Tag::new("SET")));
        }

        // Handle RESET commands
        if sql_lower.starts_with("reset ") {
            debug!("Extended query: Acknowledging RESET command: {}", sql);
            return Ok(Response::Execution(Tag::new("RESET")));
        }

        // Handle SHOW commands - same pattern as simple_query.rs
        if sql_lower.starts_with("show ") {
            debug!("Extended query: Handling SHOW command: {}", sql);
            return Ok(self.handle_show_command(&sql_lower));
        }

        // Handle empty statements (JDBC drivers may send these)
        if sql.trim().is_empty() {
            debug!("Extended query: Skipping empty statement");
            return Ok(Response::EmptyQuery);
        }

        // Fetch workspaces from storage (same pattern as simple_query.rs)
        let workspaces = self
            .storage
            .workspaces()
            .list(RepoScope::new(&context.tenant_id, &context.repository))
            .await
            .map_err(|e| {
                error!("Failed to fetch workspaces: {}", e);
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "ERROR".to_owned(),
                    "XX000".to_owned(),
                    format!("Failed to fetch workspaces: {}", e),
                )))
            })?;

        debug!(
            "Found {} workspaces: {:?}",
            workspaces.len(),
            workspaces.iter().map(|w| &w.name).collect::<Vec<_>>()
        );

        // Create catalog with all workspaces registered
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace in &workspaces {
            catalog.register_workspace(workspace.name.clone());
        }
        let catalog = Arc::new(catalog);

        // Get effective branch: session override or repository default
        let branch = match context.session_branch() {
            Some(b) => b.to_string(),
            None => {
                // Get repository's default_branch from storage
                match self
                    .storage
                    .repository_management()
                    .get_repository(&context.tenant_id, &context.repository)
                    .await
                {
                    Ok(Some(repo)) => repo.config.default_branch.clone(),
                    _ => "main".to_string(), // Fallback if repo not found
                }
            }
        };

        debug!(
            "Extended query: Using branch '{}' for query execution",
            branch
        );

        // Create QueryEngine with proper context and effective branch
        let mut engine = QueryEngine::new(
            self.storage.clone(),
            &context.tenant_id,
            &context.repository,
            &branch,
        )
        .with_catalog(catalog.clone());

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

        // Use identity auth context if present (from SET app.user)
        // Re-fetch context to get updated identity_auth after SET app.user command
        let current_context = self.auth_handler.get_context(client).ok_or_else(|| {
            error!("Lost connection context during query execution");
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "28000".to_owned(),
                "Lost connection context during query execution".to_owned(),
            )))
        })?;
        if let Some(auth) = current_context.identity_auth().cloned() {
            debug!(
                "Using identity auth context for query: actor='{}'",
                auth.actor_id()
            );
            engine = engine.with_auth(auth);
        }

        // Execute the query using execute_batch for proper support
        let row_stream = engine.execute_batch(&sql).await.map_err(|e| {
            error!("Query execution failed: {}", e);
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_owned(),
                "XX000".to_owned(), // Internal error
                format!("Query execution failed: {}", e),
            )))
        })?;

        // Collect rows from stream for schema inference
        let mut rows = Vec::new();
        futures::pin_mut!(row_stream);
        while let Some(row_result) = row_stream.next().await {
            match row_result {
                Ok(row) => rows.push(row),
                Err(e) => {
                    error!("Error reading row from stream: {}", e);
                    return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                        "ERROR".to_owned(),
                        "XX000".to_owned(),
                        format!("Failed to collect query results: {}", e),
                    ))));
                }
            }
        }

        debug!("Collected {} rows", rows.len());

        // Check if this is a SELECT query or a modification query
        let sql_upper = sql.trim().to_uppercase();
        if sql_upper.starts_with("SELECT") || sql_upper.starts_with("WITH") {
            // Query returns rows - always need proper schema
            let format_for = |idx| portal.result_column_format.format_for(idx);
            let encoder = ResultEncoder::new();

            let schema = if rows.is_empty() {
                // Empty result set - use analyzer to get schema from SQL
                debug!("Empty result set, inferring schema from SQL");
                let inferred = self.infer_schema_from_sql(&sql, &catalog)?;
                let fields: Vec<FieldInfo> = inferred
                    .iter()
                    .enumerate()
                    .map(|(idx, field)| {
                        FieldInfo::new(
                            field.name().to_string(),
                            field.table_id(),
                            field.column_id(),
                            field.datatype().clone(),
                            format_for(idx),
                        )
                    })
                    .collect();
                Arc::new(fields)
            } else {
                // Infer schema from results
                let columns = infer_schema_from_rows(&rows);
                encoder.encode_schema_with_formats(&columns, format_for)
            };

            Ok(Response::Query(ResultEncoder::build_query_response(
                rows, schema,
            )))
        } else {
            // Modification query (INSERT, UPDATE, DELETE, etc.)
            let affected_rows = rows.len();
            Ok(Response::Execution(Tag::new("OK").with_rows(affected_rows)))
        }
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        stmt: &StoredStatement<Self::Statement>,
    ) -> PgWireResult<DescribeStatementResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        debug!("Describing statement: {}", stmt.statement.sql);

        // Get parameter types from the statement
        let param_types = stmt.parameter_types.clone();

        let sql_lower = stmt.statement.sql.trim().to_lowercase();

        // Handle SHOW commands - return single TEXT column
        if sql_lower.starts_with("show ") {
            if let Some(param) = sql_lower.strip_prefix("show ") {
                let param = param.trim().trim_matches(|c| c == '\'' || c == '"');
                let field = FieldInfo::new(
                    param.to_string(),
                    None,
                    None,
                    Type::TEXT,
                    pgwire::api::results::FieldFormat::Text,
                );
                debug!("Describing SHOW statement: column '{}'", param);
                return Ok(DescribeStatementResponse::new(param_types, vec![field]));
            }
        }

        // For other statements, return empty fields (client will get schema on portal describe)
        let fields = Vec::new();

        Ok(DescribeStatementResponse::new(param_types, fields))
    }

    async fn do_describe_portal<C>(
        &self,
        client: &mut C,
        portal: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        debug!("Describing portal: {}", portal.statement.statement.sql);

        // Bind parameters to get the actual SQL
        let sql = match self.bind_parameters(portal) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to bind parameters for describe: {}", e);
                return Ok(DescribePortalResponse::new(Vec::new()));
            }
        };

        let sql_lower = sql.trim().to_lowercase();

        // Handle SHOW commands - no catalog/workspace needed
        if sql_lower.starts_with("show ") {
            if let Some(param) = sql_lower.strip_prefix("show ") {
                let param = param.trim().trim_matches(|c| c == '\'' || c == '"');
                let field = FieldInfo::new(
                    param.to_string(),
                    None,
                    None,
                    Type::TEXT,
                    pgwire::api::results::FieldFormat::Text,
                );
                debug!("Describing SHOW portal: column '{}'", param);
                return Ok(DescribePortalResponse::new(vec![field]));
            }
        }

        // Handle SET/RESET commands - no result schema
        if sql_lower.starts_with("set ") || sql_lower.starts_with("reset ") {
            debug!("Describing SET/RESET portal: no columns");
            return Ok(DescribePortalResponse::new(Vec::new()));
        }

        // Get connection context from auth handler
        let context = match self.auth_handler.get_context(client) {
            Some(ctx) => ctx,
            None => {
                warn!("No connection context for describe portal");
                return Ok(DescribePortalResponse::new(Vec::new()));
            }
        };

        // Fetch workspaces for catalog
        let workspaces = match self
            .storage
            .workspaces()
            .list(RepoScope::new(&context.tenant_id, &context.repository))
            .await
        {
            Ok(ws) => ws,
            Err(e) => {
                warn!("Failed to fetch workspaces for describe: {}", e);
                return Ok(DescribePortalResponse::new(Vec::new()));
            }
        };

        // Create catalog with workspaces
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace in &workspaces {
            catalog.register_workspace(workspace.name.clone());
        }
        let catalog = Arc::new(catalog);

        // Use analyzer to infer schema from SQL without executing
        match self.infer_schema_from_sql(&sql, &catalog) {
            Ok(fields) => {
                debug!("Inferred {} columns for portal", fields.len());
                Ok(DescribePortalResponse::new(fields.to_vec()))
            }
            Err(e) => {
                warn!("Failed to infer schema for describe: {}", e);
                Ok(DescribePortalResponse::new(Vec::new()))
            }
        }
    }
}
