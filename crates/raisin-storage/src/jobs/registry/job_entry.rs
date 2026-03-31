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

//! Internal job entry representation for the registry.

use chrono::{DateTime, Utc};
use std::sync::Arc;

use crate::jobs::{JobId, JobInfo, JobStatus, JobType};

/// Internal representation of a running job
#[derive(Clone)]
pub(super) struct JobEntry {
    pub id: JobId,
    pub job_type: JobType,
    pub status: JobStatus,
    pub tenant: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub cancel_token: Option<Arc<tokio_util::sync::CancellationToken>>,
    pub result: Option<serde_json::Value>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub timeout_seconds: u64,
    pub next_retry_at: Option<DateTime<Utc>>,
}

impl JobEntry {
    pub(super) fn to_job_info(&self) -> JobInfo {
        JobInfo {
            id: self.id.clone(),
            job_type: self.job_type.clone(),
            status: self.status.clone(),
            tenant: self.tenant.clone(),
            started_at: self.started_at,
            completed_at: self.completed_at,
            progress: self.progress,
            error: self.error.clone(),
            result: self.result.clone(),
            retry_count: self.retry_count,
            max_retries: self.max_retries,
            last_heartbeat: self.last_heartbeat,
            timeout_seconds: self.timeout_seconds,
            next_retry_at: self.next_retry_at,
        }
    }
}
