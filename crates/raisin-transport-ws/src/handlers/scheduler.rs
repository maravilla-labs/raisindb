// SPDX-License-Identifier: BSL-1.1

//! One-shot scheduled invocation handlers.
//!
//! Repo-scoped scheduling of a single future function or flow run:
//! - `scheduled_invocation_create`: register a `ScheduledInvocation` job to
//!   fire at `run_at` (delayed dispatch; restart-safe).
//! - `scheduled_invocation_cancel`: cancel by job id or external key before
//!   it fires.
//! - `scheduled_invocation_list` / `scheduled_invocation_get`: inspect
//!   pending and completed invocations.

use parking_lot::RwLock;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

// ---------------------------------------------------------------------------
// RocksDB-backed implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
mod inner {
    use super::*;
    use crate::protocol::{
        ScheduledInvocationCreatePayload, ScheduledInvocationListPayload,
        ScheduledInvocationRefPayload,
    };
    use raisin_rocksdb::{
        META_ACTOR, META_EXTERNAL_KEY, META_INPUT, META_SCHEDULED_FOR, META_TARGET_PATH,
    };
    use raisin_storage::jobs::{JobContext, JobInfo, JobStatus, JobType};
    use raisin_storage::Storage;
    use std::collections::HashMap;

    const DEFAULT_BRANCH: &str = "main";
    const FUNCTIONS_WORKSPACE: &str = "functions";

    /// Require `context.repository` from the request.
    fn require_repo(request: &RequestEnvelope) -> Result<String, WsError> {
        request
            .context
            .repository
            .clone()
            .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))
    }

    /// Extract the actor identity from the connection state.
    fn extract_actor(connection_state: &Arc<RwLock<ConnectionState>>) -> String {
        let conn = connection_state.read();
        let auth = conn.auth_context().cloned();
        drop(conn);
        auth.as_ref()
            .and_then(|a| a.user_id.clone())
            .unwrap_or_else(|| "ws_api".to_string())
    }

    /// The connection's tenant — established at upgrade time from the /sys
    /// path or x-tenant-id header (never from message content). Scheduled
    /// invocations must be registered/looked up in the caller's actual
    /// tenant, not a hardcoded one.
    fn connection_tenant(connection_state: &Arc<RwLock<ConnectionState>>) -> String {
        connection_state.read().tenant_id.clone()
    }

    fn rocksdb_handle<S, B>(
        state: &Arc<WsState<S, B>>,
    ) -> Result<Arc<raisin_rocksdb::RocksDBStorage>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| WsError::InternalError("RocksDB storage not available".to_string()))
            .cloned()
    }

    fn job_status_to_string(status: &JobStatus) -> &'static str {
        match status {
            JobStatus::Scheduled => "scheduled",
            JobStatus::Running | JobStatus::Executing => "running",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed(_) => "failed",
        }
    }

    /// Render a scheduled invocation (job + context) as its wire JSON shape.
    fn invocation_json(info: &JobInfo, context: &JobContext) -> serde_json::Value {
        let (invocation_id, target_kind) = match &info.job_type {
            JobType::ScheduledInvocation {
                invocation_id,
                target_kind,
            } => (invocation_id.clone(), target_kind.clone()),
            _ => (String::new(), String::new()),
        };
        serde_json::json!({
            "job_id": info.id.to_string(),
            "invocation_id": invocation_id,
            "target_kind": target_kind,
            "target_path": context.metadata.get(META_TARGET_PATH),
            "input": context.metadata.get(META_INPUT),
            "actor": context.metadata.get(META_ACTOR),
            "external_key": context.metadata.get(META_EXTERNAL_KEY),
            "run_at": context.metadata.get(META_SCHEDULED_FOR),
            "status": job_status_to_string(&info.status),
            "error": info.error,
            "result": info.result,
        })
    }

    /// Collect this repository's scheduled invocations (job info + context).
    async fn list_repo_invocations(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        tenant_id: &str,
        repo: &str,
    ) -> Vec<(JobInfo, JobContext)> {
        // Merge persisted + live jobs so fired/completed invocations remain
        // listable after the in-memory registry evicts them or a restart.
        let jobs = rocksdb
            .list_tenant_jobs_merged(tenant_id)
            .await
            .unwrap_or_default();
        let mut out = Vec::new();
        for job in jobs {
            if !matches!(job.job_type, JobType::ScheduledInvocation { .. }) {
                continue;
            }
            let Ok(Some(context)) = rocksdb.job_data_store().get(tenant_id, &job.id) else {
                continue;
            };
            if context.repo_id != repo {
                continue;
            }
            out.push((job, context));
        }
        out
    }

    /// Resolve a scheduled invocation by job id or external key, verifying
    /// it belongs to this repository (prevents cross-repo cancellation).
    async fn resolve_invocation(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        tenant_id: &str,
        repo: &str,
        job_id: Option<&str>,
        external_key: Option<&str>,
    ) -> Result<(JobInfo, JobContext), WsError> {
        if let Some(job_id) = job_id {
            let id = raisin_storage::jobs::JobId::from_string(job_id.to_string());
            // Falls back to the persisted store and re-hydrates trimmed
            // results, so fired invocations stay resolvable.
            let info =
                raisin_storage::BackgroundJobs::get_job_info(rocksdb.as_ref(), tenant_id, &id)
                    .await
                    .map_err(|e| WsError::OperationError(e.to_string()))?;
            if !matches!(info.job_type, JobType::ScheduledInvocation { .. }) {
                return Err(WsError::InvalidRequest(format!(
                    "Job '{}' is not a scheduled invocation",
                    job_id
                )));
            }
            let context = rocksdb
                .job_data_store()
                .get(tenant_id, &id)
                .map_err(|e| WsError::InternalError(e.to_string()))?
                .ok_or_else(|| {
                    WsError::OperationError(format!(
                        "Scheduled invocation '{}' has no context",
                        job_id
                    ))
                })?;
            if context.repo_id != repo {
                return Err(WsError::OperationError(format!(
                    "Scheduled invocation '{}' not found in repository '{}'",
                    job_id, repo
                )));
            }
            return Ok((info, context));
        }

        if let Some(key) = external_key {
            let matches = list_repo_invocations(rocksdb, tenant_id, repo).await;
            return matches
                .into_iter()
                .find(|(_, ctx)| {
                    ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str()) == Some(key)
                })
                .ok_or_else(|| {
                    WsError::OperationError(format!(
                        "No scheduled invocation with external key '{}' in repository '{}'",
                        key, repo
                    ))
                });
        }

        Err(WsError::InvalidRequest(
            "Either 'job_id' or 'external_key' is required".to_string(),
        ))
    }

    pub async fn handle_scheduled_invocation_create<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: ScheduledInvocationCreatePayload =
            serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let actor = extract_actor(connection_state);
        let tenant_id = connection_tenant(connection_state);
        let rocksdb = rocksdb_handle(state)?;

        if payload.target_kind != "function" && payload.target_kind != "flow" {
            return Err(WsError::InvalidRequest(format!(
                "Invalid target_kind '{}' (expected 'function' or 'flow')",
                payload.target_kind
            )));
        }

        let run_at = chrono::DateTime::parse_from_rfc3339(&payload.run_at)
            .map_err(|e| {
                WsError::InvalidRequest(format!(
                    "Invalid 'run_at' timestamp '{}': {} (expected RFC3339)",
                    payload.run_at, e
                ))
            })?
            .with_timezone(&chrono::Utc);

        let invocation_id = nanoid::nanoid!();
        let job_type = JobType::ScheduledInvocation {
            invocation_id: invocation_id.clone(),
            target_kind: payload.target_kind.clone(),
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            META_TARGET_PATH.to_string(),
            serde_json::json!(payload.target_path),
        );
        metadata.insert(META_INPUT.to_string(), payload.input);
        metadata.insert(META_ACTOR.to_string(), serde_json::json!(actor));
        if let Some(key) = &payload.external_key {
            metadata.insert(META_EXTERNAL_KEY.to_string(), serde_json::json!(key));
        }
        metadata.insert(
            META_SCHEDULED_FOR.to_string(),
            serde_json::json!(run_at.to_rfc3339()),
        );

        let context = JobContext {
            tenant_id: tenant_id.clone(),
            repo_id: repo.clone(),
            branch: payload.branch.unwrap_or_else(|| DEFAULT_BRANCH.to_string()),
            workspace_id: payload
                .workspace
                .unwrap_or_else(|| FUNCTIONS_WORKSPACE.to_string()),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        rocksdb
            .job_data_store()
            .put(&job_id, &context)
            .map_err(|e| WsError::InternalError(e.to_string()))?;

        // max_retries defaults to 0: a failed one-shot must not silently
        // re-fire unless the caller opts into retries.
        rocksdb
            .job_registry()
            .register_job_at_with_id(
                job_id.clone(),
                job_type,
                tenant_id.clone(),
                run_at,
                Some(payload.max_retries.unwrap_or(0)),
            )
            .await
            .map_err(|e| WsError::StorageError(e.to_string()))?;

        tracing::info!(
            job_id = %job_id,
            invocation_id = %invocation_id,
            target_kind = %payload.target_kind,
            target_path = %payload.target_path,
            run_at = %run_at,
            "Scheduled one-shot invocation via WS"
        );

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "job_id": job_id.to_string(),
                "invocation_id": invocation_id,
                "status": "scheduled",
                "run_at": run_at.to_rfc3339(),
            }),
        )))
    }

    pub async fn handle_scheduled_invocation_cancel<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: ScheduledInvocationRefPayload =
            serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let tenant_id = connection_tenant(connection_state);
        let rocksdb = rocksdb_handle(state)?;

        let (info, _context) = resolve_invocation(
            &rocksdb,
            &tenant_id,
            &repo,
            payload.job_id.as_deref(),
            payload.external_key.as_deref(),
        )
        .await?;

        rocksdb
            .job_registry()
            .cancel_job(&info.id)
            .await
            .map_err(|e| WsError::StorageError(e.to_string()))?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "job_id": info.id.to_string(),
                "status": "cancelled",
            }),
        )))
    }

    pub async fn handle_scheduled_invocation_get<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: ScheduledInvocationRefPayload =
            serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let tenant_id = connection_tenant(connection_state);
        let rocksdb = rocksdb_handle(state)?;

        let (info, context) = resolve_invocation(
            &rocksdb,
            &tenant_id,
            &repo,
            payload.job_id.as_deref(),
            payload.external_key.as_deref(),
        )
        .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            invocation_json(&info, &context),
        )))
    }

    pub async fn handle_scheduled_invocation_list<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: ScheduledInvocationListPayload = if request.payload.is_null() {
            ScheduledInvocationListPayload::default()
        } else {
            serde_json::from_value(request.payload.clone())?
        };
        let repo = require_repo(&request)?;
        let tenant_id = connection_tenant(connection_state);
        let rocksdb = rocksdb_handle(state)?;

        let invocations = list_repo_invocations(&rocksdb, &tenant_id, &repo).await;
        let items: Vec<serde_json::Value> = invocations
            .iter()
            .filter(|(info, ctx)| {
                if let Some(key) = &payload.external_key {
                    if ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str())
                        != Some(key.as_str())
                    {
                        return false;
                    }
                }
                if let Some(status) = &payload.status {
                    if job_status_to_string(&info.status) != status {
                        return false;
                    }
                }
                true
            })
            .map(|(info, ctx)| invocation_json(info, ctx))
            .collect();

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({ "invocations": items }),
        )))
    }
}

// ---------------------------------------------------------------------------
// Feature-gated re-exports / fallback stubs
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
pub use inner::{
    handle_scheduled_invocation_cancel, handle_scheduled_invocation_create,
    handle_scheduled_invocation_get, handle_scheduled_invocation_list,
};

#[cfg(not(feature = "storage-rocksdb"))]
macro_rules! scheduler_stub {
    ($name:ident) => {
        pub async fn $name<S, B>(
            _state: &Arc<WsState<S, B>>,
            _connection_state: &Arc<RwLock<ConnectionState>>,
            request: RequestEnvelope,
        ) -> Result<Option<ResponseEnvelope>, WsError>
        where
            S: raisin_storage::Storage
                + raisin_storage::transactional::TransactionalStorage
                + 'static,
            B: raisin_binary::BinaryStorage + 'static,
        {
            Ok(Some(ResponseEnvelope::error(
                request.request_id,
                "NOT_IMPLEMENTED".to_string(),
                "Scheduled invocations require RocksDB backend".to_string(),
            )))
        }
    };
}

#[cfg(not(feature = "storage-rocksdb"))]
scheduler_stub!(handle_scheduled_invocation_create);
#[cfg(not(feature = "storage-rocksdb"))]
scheduler_stub!(handle_scheduled_invocation_cancel);
#[cfg(not(feature = "storage-rocksdb"))]
scheduler_stub!(handle_scheduled_invocation_get);
#[cfg(not(feature = "storage-rocksdb"))]
scheduler_stub!(handle_scheduled_invocation_list);
