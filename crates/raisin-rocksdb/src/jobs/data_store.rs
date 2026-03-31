//! Job context data store implementation using RocksDB
//!
//! This module provides persistent storage for JobContext data, which contains
//! execution context (tenant_id, repo_id, branch, workspace_id, revision, metadata)
//! needed by workers to process jobs. JobContext is stored separately from JobInfo
//! to keep JobInfo lightweight.

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

    /// Store job context data
    ///
    /// # Arguments
    /// * `job_id` - Unique identifier for the job
    /// * `context` - Job execution context to store
    ///
    /// # Errors
    /// Returns an error if serialization fails or database write fails
    pub fn put(&self, job_id: &JobId, context: &JobContext) -> Result<()> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_id.as_str().as_bytes();
        let value = rmp_serde::to_vec(context).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize job context: {}", e))
        })?;

        self.db.put_cf(cf, key, value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to store job context: {}", e))
        })?;

        Ok(())
    }

    /// Retrieve job context data
    ///
    /// # Arguments
    /// * `job_id` - Unique identifier for the job
    ///
    /// # Returns
    /// Returns `Some(JobContext)` if found, `None` if not found
    ///
    /// # Errors
    /// Returns an error if database read fails or deserialization fails
    pub fn get(&self, job_id: &JobId) -> Result<Option<JobContext>> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_id.as_str().as_bytes();

        let value = self.db.get_cf(cf, key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read job context: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let context = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize job context: {}",
                        e
                    ))
                })?;
                Ok(Some(context))
            }
            None => Ok(None),
        }
    }

    /// Delete job context data (cleanup after job completion)
    ///
    /// # Arguments
    /// * `job_id` - Unique identifier for the job
    ///
    /// # Errors
    /// Returns an error if database delete fails
    pub fn delete(&self, job_id: &JobId) -> Result<()> {
        let cf = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_id.as_str().as_bytes();

        self.db.delete_cf(cf, key).map_err(|e| {
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
        let result = store.get(&job_id).unwrap();
        assert!(result.is_none());

        // Store context
        store.put(&job_id, &context).unwrap();

        // Retrieve context
        let result = store.get(&job_id).unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.tenant_id, context.tenant_id);
        assert_eq!(retrieved.repo_id, context.repo_id);
        assert_eq!(retrieved.branch, context.branch);
        assert_eq!(retrieved.workspace_id, context.workspace_id);
        assert_eq!(retrieved.revision, context.revision);
        assert_eq!(retrieved.metadata.len(), context.metadata.len());

        // Delete context
        store.delete(&job_id).unwrap();

        // Context should no longer exist
        let result = store.get(&job_id).unwrap();
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
        let result = store.get(&job_id).unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.revision, raisin_hlc::HLC::new(2, 0));
    }

    #[test]
    fn test_delete_nonexistent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        let store = JobDataStore::new(Arc::new(db));

        let job_id = JobId::new();

        // Deleting non-existent context should succeed (idempotent)
        let result = store.delete(&job_id);
        assert!(result.is_ok());
    }
}
