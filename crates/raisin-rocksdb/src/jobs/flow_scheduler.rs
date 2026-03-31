// SPDX-License-Identifier: BSL-1.1

//! Implementation of [`FlowJobScheduler`] for [`RocksDBStorage`].

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_flow_runtime::service::FlowJobScheduler;
use raisin_flow_runtime::types::FlowError;
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobType};

use crate::storage::RocksDBStorage;

const TENANT_ID: &str = "default";
const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";

#[async_trait]
impl FlowJobScheduler for RocksDBStorage {
    async fn schedule_flow_job(
        &self,
        repo: &str,
        job_type: JobType,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<String, FlowError> {
        let context = JobContext {
            tenant_id: TENANT_ID.to_string(),
            repo_id: repo.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            workspace_id: FUNCTIONS_WORKSPACE.to_string(),
            revision: HLC::new(0, 0),
            metadata,
        };

        let job_id = self
            .job_registry()
            .register_job(job_type, Some(TENANT_ID.to_string()), None, None, None)
            .await
            .map_err(|e| FlowError::Other(e.to_string()))?;

        self.job_data_store()
            .put(&job_id, &context)
            .map_err(|e| FlowError::Other(e.to_string()))?;

        Ok(job_id.to_string())
    }

    async fn cancel_flow_jobs(&self, instance_id: &str) -> Result<(), FlowError> {
        let registry = self.job_registry();
        let jobs = registry.list_jobs().await;
        for job in jobs {
            if let JobType::FlowInstanceExecution {
                instance_id: ref job_instance_id,
                ..
            } = job.job_type
            {
                if job_instance_id == instance_id {
                    let _ = registry.cancel_job(&job.id).await;
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Get a `&dyn FlowJobScheduler` from an `Option<Arc<RocksDBStorage>>`.
pub fn get_flow_job_scheduler(
    rocksdb: &Option<Arc<RocksDBStorage>>,
) -> Result<&dyn FlowJobScheduler, FlowError> {
    rocksdb
        .as_ref()
        .map(|s| s.as_ref() as &dyn FlowJobScheduler)
        .ok_or_else(|| FlowError::NotSupported("RocksDB storage not available".to_string()))
}
