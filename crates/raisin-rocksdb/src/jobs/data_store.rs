//! Job context data store implementation using RocksDB
//!
//! This module provides persistent storage for JobContext data, which contains
//! execution context (tenant_id, repo_id, branch, workspace_id, revision, metadata)
//! needed by workers to process jobs. JobContext is stored separately from JobInfo
//! to keep JobInfo lightweight.

use crate::keys::job_key;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobId};
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB-backed job context data store
///
/// This implementation stores JobContext data keyed by job_id in the JOB_DATA
/// column family. Data is serialized using messagepack for efficient storage.
#[derive(Clone)]
pub struct JobDataStore {
    db: Arc<DB>,
}

impl JobDataStore {
    /// Create a new job data store
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Store job context data.
    ///
    /// Key layout is `{tenant}\0{job_id}` so prefix scans / range deletes are
    /// tenant-scoped. The tenant is sourced from the supplied `JobContext`.
    pub fn put(&self, job_id: &JobId, context: &JobContext) -> Result<()> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_key(&context.tenant_id, job_id.as_str());
        let value = rmp_serde::to_vec(context).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize job context: {}", e))
        })?;

        self.db.put_cf(cf, &key, value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to store job context: {}", e))
        })?;

        Ok(())
    }

    /// Retrieve job context data for the given tenant.
    ///
    /// Defense-in-depth: the key already includes the tenant prefix, but we
    /// also verify that the deserialized `JobContext.tenant_id` matches the
    /// lookup tenant. If they disagree, something has gone wrong upstream
    /// (mismatched tenants between `register_job` and `JobDataStore::put`),
    /// so log a structured error and return `None` rather than handing a
    /// foreign-tenant context to the worker.
    pub fn get(&self, tenant: &str, job_id: &JobId) -> Result<Option<JobContext>> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_key(tenant, job_id.as_str());

        let value = self.db.get_cf(cf, &key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read job context: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let context: JobContext = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize job context: {}",
                        e
                    ))
                })?;

                if context.tenant_id != tenant {
                    tracing::error!(
                        lookup_tenant = %tenant,
                        stored_tenant = %context.tenant_id,
                        job_id = %job_id,
                        "JobContext tenant mismatch: stored context's tenant_id does \
                         not match the lookup tenant. Returning None to avoid leaking \
                         cross-tenant context to a worker."
                    );
                    return Ok(None);
                }

                Ok(Some(context))
            }
            None => Ok(None),
        }
    }

    /// Delete job context data (cleanup after job completion).
    pub fn delete(&self, tenant: &str, job_id: &JobId) -> Result<()> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_key(tenant, job_id.as_str());

        self.db.delete_cf(cf, &key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to delete job context: {}", e))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_context() -> JobContext {
        JobContext {
            tenant_id: "test-tenant".to_string(),
            repo_id: "test-repo".to_string(),
            branch: "main".to_string(),
            workspace_id: "test-workspace".to_string(),
            revision: raisin_hlc::HLC::new(42, 0),
            metadata: {
                let mut map = HashMap::new();
                map.insert("key1".to_string(), serde_json::json!("value1"));
                map.insert("key2".to_string(), serde_json::json!(123));
                map
            },
        }
    }

    #[test]
    fn test_put_get_delete() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        let store = JobDataStore::new(Arc::new(db));

        let job_id = JobId::new();
        let context = create_test_context();

        // Initially, context should not exist
        let result = store.get(&context.tenant_id, &job_id).unwrap();
        assert!(result.is_none());

        // Store context
        store.put(&job_id, &context).unwrap();

        // Retrieve context
        let result = store.get(&context.tenant_id, &job_id).unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.tenant_id, context.tenant_id);
        assert_eq!(retrieved.repo_id, context.repo_id);
        assert_eq!(retrieved.branch, context.branch);
        assert_eq!(retrieved.workspace_id, context.workspace_id);
        assert_eq!(retrieved.revision, context.revision);
        assert_eq!(retrieved.metadata.len(), context.metadata.len());

        // Delete context
        store.delete(&context.tenant_id, &job_id).unwrap();

        // Context should no longer exist
        let result = store.get(&context.tenant_id, &job_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_overwrite_context() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        let store = JobDataStore::new(Arc::new(db));

        let job_id = JobId::new();
        let mut context1 = create_test_context();
        context1.revision = raisin_hlc::HLC::new(1, 0);

        let mut context2 = create_test_context();
        context2.revision = raisin_hlc::HLC::new(2, 0);

        // Store first context
        store.put(&job_id, &context1).unwrap();

        // Overwrite with second context
        store.put(&job_id, &context2).unwrap();

        // Should retrieve second context
        let result = store.get(&context1.tenant_id, &job_id).unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.revision, raisin_hlc::HLC::new(2, 0));
    }

    #[test]
    fn test_get_returns_none_on_tenant_mismatch() {
        // Defense-in-depth: if a stored JobContext somehow ended up with a
        // tenant_id that disagrees with the key prefix it was stored under,
        // `get` should refuse to return it (and log) rather than hand a
        // foreign-tenant context to a worker.
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
        let store = JobDataStore::new(Arc::clone(&db));

        let job_id = JobId::new();

        // Forge a key for tenant "a" but a value whose tenant_id is "b".
        let mut bad_context = create_test_context();
        bad_context.tenant_id = "b".to_string();
        let cf = cf_handle(&db, cf::JOB_DATA).unwrap();
        let key = crate::keys::job_key("a", job_id.as_str());
        let bytes = rmp_serde::to_vec(&bad_context).unwrap();
        db.put_cf(cf, &key, bytes).unwrap();

        // Looking up under tenant "a" must NOT return the cross-tenant context.
        let result = store.get("a", &job_id).unwrap();
        assert!(result.is_none(), "tenant-mismatched context should be filtered out");

        // Looking up under the stored tenant "b" simply has no entry (the
        // forged value is keyed under "a"), so it returns None too.
        let result = store.get("b", &job_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        let store = JobDataStore::new(Arc::new(db));

        let job_id = JobId::new();

        // Deleting non-existent context should succeed (idempotent)
        let result = store.delete("test-tenant", &job_id);
        assert!(result.is_ok());
    }
}
