// SPDX-License-Identifier: BSL-1.1

//! Background job enqueueing for package uploads.

use crate::state::AppState;

/// Enqueue a PackageProcess background job for package uploads.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn enqueue_package_process_job(
    state: &AppState,
    created_node_id: &str,
    stored: &raisin_binary::StoredObject,
    ws: &str,
    tenant_id: &str,
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
            repo_id: "default".to_string(),
            branch: "main".to_string(),
            workspace_id: ws.to_string(),
            revision: raisin_hlc::HLC::now(),
            metadata,
        };

        match job_registry
            .register_job(job_type, Some(tenant_id.to_string()), None, None, None)
            .await
        {
            Ok(job_id) => {
                if let Err(e) = job_data_store.put(&job_id, &job_context) {
                    tracing::warn!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to store job context for package processing"
                    );
                } else {
                    tracing::info!(
                        job_id = %job_id,
                        package_node_id = %created_node_id,
                        "Enqueued PackageProcess job"
                    );
                }
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
