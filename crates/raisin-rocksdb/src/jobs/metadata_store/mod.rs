//! Job metadata persistence using RocksDB
//!
//! This module stores JobEntry metadata (job type, status, timestamps, retry info)
//! in the JOB_METADATA column family for crash recovery and job history.
//! Uses WriteBatch for atomic operations with JobContext.

mod cleanup;
mod crud;
mod persistence_impl;
mod queries;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use raisin_storage::jobs::{JobId, JobStatus, JobType};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Persisted representation of job metadata (without runtime-only fields)
///
/// This struct is serialized to RocksDB and contains all job information
/// needed for crash recovery, retry, and history. It excludes the cancel_token
/// which is runtime-only and can't be serialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedJobEntry {
    pub id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    pub tenant: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub result: Option<serde_json::Value>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub next_retry_at: Option<DateTime<Utc>>,
}

/// RocksDB-backed job metadata store
///
/// Provides persistence for job metadata with atomic operations using WriteBatch.
/// Jobs are keyed by job_id in the JOB_METADATA column family.
#[derive(Clone)]
pub struct JobMetadataStore {
    pub(super) db: Arc<DB>,
}

impl JobMetadataStore {
    /// Create a new job metadata store
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}
