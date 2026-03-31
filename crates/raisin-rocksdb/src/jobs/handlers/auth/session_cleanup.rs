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

//! Session cleanup handler

use async_trait::async_trait;
use raisin_auth::jobs::{SessionCleanupConfig, SessionCleanupResult};
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback trait for session store operations
#[async_trait]
pub trait SessionCleanupStore: Send + Sync {
    /// Find expired sessions
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Optional tenant to scope the search (None = all tenants)
    /// * `max_idle_seconds` - Maximum idle time before session is considered expired
    /// * `batch_size` - Maximum number of sessions to return
    ///
    /// # Returns
    ///
    /// List of expired session IDs
    async fn find_expired_sessions(
        &self,
        tenant_id: Option<&str>,
        max_idle_seconds: Option<u64>,
        batch_size: usize,
    ) -> Result<Vec<String>>;

    /// Delete a session by ID
    async fn delete_session(&self, session_id: &str) -> Result<()>;

    /// Invalidate cache entries for a session
    async fn invalidate_cache(&self, session_id: &str) -> Result<()>;
}

/// Handler for session cleanup jobs
pub struct AuthSessionCleanupHandler<S: SessionCleanupStore> {
    session_store: Arc<S>,
}

impl<S: SessionCleanupStore> AuthSessionCleanupHandler<S> {
    /// Create a new session cleanup handler
    pub fn new(session_store: Arc<S>) -> Self {
        Self { session_store }
    }

    /// Handle session cleanup job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<SessionCleanupResult> {
        // Verify job type and extract tenant
        let tenant_id = match &job.job_type {
            JobType::AuthSessionCleanup { tenant_id, .. } => tenant_id.clone(),
            _ => {
                return Err(Error::Validation(
                    "Expected AuthSessionCleanup job type".to_string(),
                ))
            }
        };

        // Parse config from context metadata
        let config = SessionCleanupConfig::from_metadata(&context.metadata);

        tracing::info!(
            job_id = %job.id,
            tenant_id = ?tenant_id,
            batch_size = config.batch_size,
            "Starting session cleanup job"
        );

        let mut result = SessionCleanupResult::new();

        // Find expired sessions
        let expired_sessions = self
            .session_store
            .find_expired_sessions(
                tenant_id.as_deref(),
                config.max_idle_seconds,
                config.batch_size,
            )
            .await?;

        result.scanned(expired_sessions.len());

        // Check if there might be more
        result.set_has_more(expired_sessions.len() >= config.batch_size);

        // Delete each expired session
        for session_id in &expired_sessions {
            match self.session_store.delete_session(session_id).await {
                Ok(()) => {
                    result.deleted(1);

                    // Invalidate cache if configured
                    if config.invalidate_cache {
                        if let Err(e) = self.session_store.invalidate_cache(session_id).await {
                            result.add_error(format!(
                                "Failed to invalidate cache for session {}: {}",
                                session_id, e
                            ));
                        } else {
                            result.invalidated(1);
                        }
                    }

                    if config.verbose_logging {
                        tracing::debug!(session_id = %session_id, "Deleted expired session");
                    }
                }
                Err(e) => {
                    result.add_error(format!("Failed to delete session {}: {}", session_id, e));
                }
            }
        }

        tracing::info!(
            job_id = %job.id,
            sessions_scanned = result.sessions_scanned,
            sessions_deleted = result.sessions_deleted,
            cache_entries_invalidated = result.cache_entries_invalidated,
            has_more = result.has_more,
            errors = result.errors.len(),
            "Session cleanup job completed"
        );

        Ok(result)
    }
}
