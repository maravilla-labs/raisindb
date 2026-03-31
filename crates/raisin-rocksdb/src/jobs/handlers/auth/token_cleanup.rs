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

//! Token cleanup handler

use async_trait::async_trait;
use raisin_auth::jobs::{TokenCleanupConfig, TokenCleanupResult};
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback trait for token store operations
#[async_trait]
pub trait TokenCleanupStore: Send + Sync {
    /// Find expired tokens
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Optional tenant to scope the search (None = all tenants)
    /// * `token_types` - Types of tokens to clean up
    /// * `grace_period_seconds` - Grace period after expiration before deletion
    /// * `batch_size` - Maximum number of tokens to return
    ///
    /// # Returns
    ///
    /// List of (token_hash, token_type) pairs for expired tokens
    async fn find_expired_tokens(
        &self,
        tenant_id: Option<&str>,
        token_types: &[String],
        grace_period_seconds: u64,
        batch_size: usize,
    ) -> Result<Vec<(String, String)>>;

    /// Delete a token by hash
    async fn delete_token(&self, token_hash: &str) -> Result<()>;
}

/// Handler for token cleanup jobs
pub struct AuthTokenCleanupHandler<S: TokenCleanupStore> {
    token_store: Arc<S>,
}

impl<S: TokenCleanupStore> AuthTokenCleanupHandler<S> {
    /// Create a new token cleanup handler
    pub fn new(token_store: Arc<S>) -> Self {
        Self { token_store }
    }

    /// Handle token cleanup job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<TokenCleanupResult> {
        // Verify job type and extract parameters
        let tenant_id = match &job.job_type {
            JobType::AuthTokenCleanup { tenant_id, .. } => tenant_id.clone(),
            _ => {
                return Err(Error::Validation(
                    "Expected AuthTokenCleanup job type".to_string(),
                ))
            }
        };

        // Parse config from context metadata
        let config = TokenCleanupConfig::from_metadata(&context.metadata);

        let token_type_strs: Vec<String> =
            config.token_types.iter().map(|t| t.to_string()).collect();

        tracing::info!(
            job_id = %job.id,
            tenant_id = ?tenant_id,
            batch_size = config.batch_size,
            token_types = ?token_type_strs,
            "Starting token cleanup job"
        );

        let mut result = TokenCleanupResult::new();

        // Find expired tokens
        let expired_tokens = self
            .token_store
            .find_expired_tokens(
                tenant_id.as_deref(),
                &token_type_strs,
                config.grace_period_seconds,
                config.batch_size,
            )
            .await?;

        result.scanned(expired_tokens.len());

        // Check if there might be more
        result.set_has_more(expired_tokens.len() >= config.batch_size);

        // Delete each expired token
        for (token_hash, token_type) in &expired_tokens {
            match self.token_store.delete_token(token_hash).await {
                Ok(()) => {
                    result.deleted(token_type);

                    if config.verbose_logging {
                        tracing::debug!(
                            token_hash = %token_hash,
                            token_type = %token_type,
                            "Deleted expired token"
                        );
                    }
                }
                Err(e) => {
                    result.add_error(format!("Failed to delete token {}: {}", token_hash, e));
                }
            }
        }

        tracing::info!(
            job_id = %job.id,
            tokens_scanned = result.tokens_scanned,
            tokens_deleted = result.tokens_deleted,
            deleted_by_type = ?result.deleted_by_type,
            has_more = result.has_more,
            errors = result.errors.len(),
            "Token cleanup job completed"
        );

        Ok(result)
    }
}
