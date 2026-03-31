// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Job info, handle, and context types

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::task::JoinHandle;

use super::id::JobId;
use super::job_type::JobType;
use super::status::JobStatus;

/// Information about a background job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: JobId,
    pub job_type: JobType,
    pub status: JobStatus,
    pub tenant: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
    /// Current retry attempt (0-based, 0 = first attempt)
    pub retry_count: u32,
    /// Maximum number of retry attempts (default 3)
    pub max_retries: u32,
    /// Last heartbeat timestamp (for timeout detection)
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
    /// Timeout in seconds (default 300 = 5 minutes)
    pub timeout_seconds: u64,
    /// When the job should be retried
    pub next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Handle to background job system
#[derive(Debug)]
pub enum JobHandle {
    /// Background jobs are disabled
    Disabled,
    /// Background jobs are running with a handle to the task
    Running(JoinHandle<()>),
}

/// Context data for a job (stored separately in job_data CF)
///
/// This contains the execution context needed by workers to process jobs.
/// Stored in RocksDB keyed by job_id to keep JobInfo lightweight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobContext {
    /// Tenant identifier
    pub tenant_id: String,
    /// Repository identifier
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Workspace identifier
    pub workspace_id: String,
    /// Revision number
    pub revision: HLC,
    /// Additional metadata for job-specific data
    pub metadata: HashMap<String, serde_json::Value>,
}
