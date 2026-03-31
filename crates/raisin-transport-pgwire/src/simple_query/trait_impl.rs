// SPDX-License-Identifier: BSL-1.1

//! SimpleQueryHandler trait implementation for pgwire.

use crate::auth::ApiKeyValidator;
use async_trait::async_trait;
use futures::sink::Sink;
use pgwire::api::query::SimpleQueryHandler;
use pgwire::api::results::Response;
use pgwire::api::ClientInfo;
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use raisin_storage::{RepositoryManagementRepository, Storage};
use std::fmt::Debug;
use tracing::{debug, error, info};

use super::handler::RaisinSimpleQueryHandler;

#[async_trait]
impl<S, V, P> SimpleQueryHandler for RaisinSimpleQueryHandler<S, V, P>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    V: ApiKeyValidator + 'static,
    P: pgwire::api::auth::ServerParameterProvider + 'static,
{
    async fn do_query<'a, 'b: 'a, C>(
        &'b self,
        client: &mut C,
        query: &'a str,
    ) -> PgWireResult<Vec<Response<'a>>>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        info!("Received simple query from {}", client.socket_addr());
        debug!("Query: {}", query);

        // Get connection context from auth handler
        let context = self.auth_handler.get_context(client).ok_or_else(|| {
            error!("No connection context found - client not authenticated");
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "FATAL".to_string(),
                "28000".to_string(),
                "Connection not authenticated".to_string(),
            )))
        })?;

        debug!(
            "Connection context: tenant={}, repo={}, user={}",
            context.tenant_id, context.repository, context.user_id
        );

        // Split query into individual statements (by semicolon)
        let statements = Self::split_statements(query);

        if statements.is_empty() {
            debug!("Empty query received");
            return Ok(vec![Response::EmptyQuery]);
        }

        info!("Processing {} statement(s)", statements.len());

        // Process each statement
        let mut responses = Vec::new();

        for (idx, sql) in statements.iter().enumerate() {
            debug!(
                "Processing statement {}/{}: {}",
                idx + 1,
                statements.len(),
                sql
            );

            // Check if this is a system query
            if let Some(system_response) = self.handle_system_query(sql, client, &context).await {
                debug!("System query handled");
                responses.push(system_response);
                continue;
            }

            // Re-fetch context to get updated identity_auth and session_branch
            let current_context = self.auth_handler.get_context(client).ok_or_else(|| {
                error!("Connection context lost during query execution");
                PgWireError::UserError(Box::new(ErrorInfo::new(
                    "FATAL".to_string(),
                    "28000".to_string(),
                    "Connection context lost".to_string(),
                )))
            })?;

            // Get effective branch: session override or repository default
            let branch = match current_context.session_branch() {
                Some(b) => b.to_string(),
                None => {
                    match self
                        .storage
                        .repository_management()
                        .get_repository(&current_context.tenant_id, &current_context.repository)
                        .await
                    {
                        Ok(Some(repo)) => repo.config.default_branch.clone(),
                        _ => "main".to_string(),
                    }
                }
            };

            // Execute the query using QueryEngine with effective branch
            let response = self
                .execute_query(
                    sql,
                    &current_context.tenant_id,
                    &current_context.repository,
                    &branch,
                    current_context.identity_auth().cloned(),
                )
                .await;

            responses.push(response);
        }

        debug!("Returning {} response(s)", responses.len());
        Ok(responses)
    }
}
