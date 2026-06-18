//! Bulk SQL execution job handler
//!
//! This module handles background SQL batch execution for operations with
//! complex WHERE clauses that require table scans. When a SQL batch contains
//! UPDATE or DELETE with WHERE clauses other than `id = 'xxx'` or `path = '/xxx'`,
//! the batch is executed asynchronously via this handler.
//!
//! # Architecture Note
//!
//! Due to dependency structure (raisin-sql-execution depends on raisin-rocksdb),
//! the actual SQL execution is done by a callback provided at runtime rather than
//! directly using QueryEngine here. The transport layer (HTTP/WS) provides the
//! executor callback when starting the job system.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback type for SQL execution
///
/// This callback is provided by the transport layer which has access to QueryEngine.
/// It takes `(sql, tenant_id, repo_id, branch, actor, auth_context)` and returns the
/// number of affected rows. `auth_context` is the identity that submitted the
/// original (synchronous) request, captured at enqueue time so the deferred bulk
/// write runs under the same RLS — never escalated to system.
pub type SqlExecutorCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
            Option<AuthContext>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + Send>>
        + Send
        + Sync,
>;

/// Handler for bulk SQL execution jobs
///
/// This handler processes BulkSql jobs. Due to the dependency structure,
/// the actual SQL execution is delegated to a callback provided by the
/// transport layer which has access to QueryEngine.
#[derive(Default)]
pub struct BulkSqlHandler {
    /// Optional SQL executor callback (set by transport layer)
    executor: Option<SqlExecutorCallback>,
}

impl BulkSqlHandler {
    /// Create a new bulk SQL job handler
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the SQL executor callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the QueryEngine-based executor.
    pub fn with_executor(mut self, executor: SqlExecutorCallback) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Handle bulk SQL execution job
    ///
    /// If no executor is configured, returns an error indicating that
    /// bulk SQL execution is not available.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::BulkSql variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract SQL and actor from JobType
        let (sql, actor) = match &job.job_type {
            JobType::BulkSql { sql, actor } => (sql.clone(), actor.clone()),
            _ => return Err(Error::Validation("Expected BulkSql job type".to_string())),
        };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            actor = %actor,
            sql_length = sql.len(),
            "Processing bulk SQL execution job"
        );

        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Bulk SQL executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Log SQL preview (truncated for long queries)
        let sql_preview = if sql.len() > 200 {
            format!("{}...", &sql[..200])
        } else {
            sql.clone()
        };
        tracing::debug!(sql = %sql_preview, "SQL batch to execute");

        // Recover the submitter's auth context (stored at enqueue time) so the
        // deferred write runs with the same RLS as the original request. If it
        // is missing we deliberately do NOT fall back to system — the executor
        // runs unprivileged (anonymous) so a lost identity fails closed.
        let auth_context = context
            .metadata
            .get("auth_context")
            .and_then(|v| serde_json::from_value::<AuthContext>(v.clone()).ok());

        if auth_context.is_none() {
            tracing::warn!(
                job_id = %job.id,
                "BulkSql job has no stored auth context; running unprivileged"
            );
        }

        // Execute via callback
        let start = std::time::Instant::now();
        let affected_rows = executor(
            sql,
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
            actor,
            auth_context,
        )
        .await?;

        let elapsed = start.elapsed();

        tracing::info!(
            job_id = %job.id,
            affected_rows = affected_rows,
            elapsed_ms = elapsed.as_millis(),
            "Bulk SQL execution completed"
        );

        Ok(())
    }
}
