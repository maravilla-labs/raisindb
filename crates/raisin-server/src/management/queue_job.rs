//! Queue an operator maintenance job so the worker pool runs it.
//!
//! The `/management/*/start` endpoints used to register a job on a registry the
//! workers do not read and then do nothing (`TODO: Re-implement`), so every one
//! returned a job id that never ran. This writes the job's context FIRST and
//! registers under that id on the storage's own registry, so the worker that
//! claims it finds its context and `MaintenanceJobHandler` runs the operation.

#[cfg(feature = "storage-rocksdb")]
pub(crate) async fn queue_maintenance_job(
    storage: &raisin_rocksdb::RocksDBStorage,
    job_type: raisin_storage::jobs::JobType,
    tenant: &str,
    metadata: std::collections::HashMap<String, serde_json::Value>,
) -> Result<raisin_storage::jobs::JobId, axum::http::StatusCode> {
    use axum::http::StatusCode;
    use raisin_storage::jobs::{JobContext, JobId};

    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: String::new(),
        branch: String::new(),
        workspace_id: String::new(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata,
    };
    let job_id = JobId::new();
    storage
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| {
            tracing::error!(error = %e, tenant, "failed to store maintenance job context");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    storage
        .job_registry()
        .register_job_with_id(job_id, job_type, tenant.to_string(), None, None, Some(0))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, tenant, "failed to register maintenance job");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}
