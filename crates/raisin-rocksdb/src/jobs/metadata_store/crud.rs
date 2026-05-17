//! Core CRUD operations for job metadata

use super::{JobMetadataStore, PersistedJobEntry};
use crate::keys::job_key;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobId};
use rocksdb::WriteBatch;

impl JobMetadataStore {
    /// Store job metadata atomically with JobContext using WriteBatch.
    ///
    /// Both keys are tenant-prefixed (`{tenant}\0{job_id}`); the tenant is
    /// sourced from the supplied `JobContext`.
    pub fn put_with_context(
        &self,
        job_id: &JobId,
        entry: &PersistedJobEntry,
        context: &JobContext,
    ) -> Result<()> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let key = job_key(&context.tenant_id, job_id.as_str());

        // Serialize both metadata and context
        let metadata_value = rmp_serde::to_vec(entry).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize job metadata: {}", e))
        })?;

        let context_value = rmp_serde::to_vec(context).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize job context: {}", e))
        })?;

        // Atomic write using WriteBatch
        let mut batch = WriteBatch::default();
        batch.put_cf(cf_metadata, &key, metadata_value);
        batch.put_cf(cf_data, &key, context_value);

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to write job metadata and context: {}", e))
        })?;

        Ok(())
    }

    /// Update job metadata (status changes, heartbeat updates, retry increments).
    ///
    /// The tenant is sourced from `entry.tenant`.
    pub fn update(&self, job_id: &JobId, entry: &PersistedJobEntry) -> Result<()> {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let key = job_key(&entry.tenant, job_id.as_str());

        let value = rmp_serde::to_vec(entry).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize job metadata: {}", e))
        })?;

        self.db.put_cf(cf, &key, value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to update job metadata: {}", e))
        })?;

        Ok(())
    }

    /// Retrieve job metadata for the given tenant + job id.
    pub fn get(&self, tenant: &str, job_id: &JobId) -> Result<Option<PersistedJobEntry>> {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let key = job_key(tenant, job_id.as_str());

        let value = self.db.get_cf(cf, &key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read job metadata: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let entry = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize job metadata: {}",
                        e
                    ))
                })?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }
}
