// SPDX-License-Identifier: BSL-1.1

//! Background job enqueueing for package uploads.

use crate::state::AppState;

/// Enqueue a PackageProcess background job for package uploads.
#[cfg(feature = "storage-rocksdb")]
#[allow(clippy::too_many_arguments)]
pub(super) async fn enqueue_package_process_job(
    state: &AppState,
    created_node_id: &str,
    stored: &raisin_binary::StoredObject,
    ws: &str,
    tenant_id: &str,
    repo: &str,
    branch: &str,
) {
    if let Some(rocksdb) = state.rocksdb_storage.as_ref() {
        let job_registry = rocksdb.job_registry();
        let job_data_store = rocksdb.job_data_store();

        let job_type = raisin_storage::jobs::JobType::PackageProcess {
            package_node_id: created_node_id.to_string(),
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "resource_key".to_string(),
            serde_json::json!(stored.key.clone()),
        );

        let job_context = raisin_storage::jobs::JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.to_string(),
            branch: branch.to_string(),
            workspace_id: ws.to_string(),
            revision: raisin_hlc::HLC::now(),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        if let Err(e) = job_data_store.put(&job_id, &job_context) {
            tracing::warn!(
                job_id = %job_id,
                error = %e,
                "Failed to store job context for package processing"
            );
            return;
        }

        match job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type,
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await
        {
            Ok(_) => {
                tracing::info!(
                    job_id = %job_id,
                    package_node_id = %created_node_id,
                    "Enqueued PackageProcess job"
                );
            }
            Err(e) => {
                tracing::warn!(
                    package_node_id = %created_node_id,
                    error = %e,
                    "Failed to register PackageProcess job"
                );
            }
        }
    }
}
