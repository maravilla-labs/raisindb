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

//! Generic worker trait for processing background jobs

use crate::jobs::{JobContext, JobInfo};
use async_trait::async_trait;
use raisin_error::Result;

/// Trait for processing individual jobs
///
/// Implementations handle specific job types by dispatching to appropriate handlers.
#[async_trait]
pub trait JobWorker: Send + Sync {
    /// Process a single job with its context
    ///
    /// # Arguments
    ///
    /// * `job` - Job metadata and status
    /// * `context` - Execution context (tenant, repo, branch, workspace, revision)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Job completed successfully
    /// * `Err(e)` - Job failed with error
    async fn process_job(&self, job: &JobInfo, context: &JobContext) -> Result<()>;

    /// Check if this worker can handle the given job type
    ///
    /// This allows for specialized workers that only handle specific job types.
    fn can_handle(&self, job_info: &JobInfo) -> bool;
}
