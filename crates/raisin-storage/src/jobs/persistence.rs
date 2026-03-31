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

//! Job persistence trait for storage backends
//!
//! This trait provides an abstraction for persisting job metadata to storage.
//! Storage backends (like RocksDB) implement this trait to enable crash recovery
//! and job history tracking without creating a direct dependency from raisin-storage
//! to specific storage implementations.

use crate::jobs::{JobId, JobInfo};
use async_trait::async_trait;
use raisin_error::Result;

/// Trait for persisting job metadata
///
/// Storage backends implement this trait to persist job state changes to durable storage.
/// This enables:
/// - Crash recovery: Restore pending jobs after a server restart
/// - Job history: Track completed/failed jobs for auditing
/// - Retry logic: Persist retry counts and error information
///
/// # Implementation Notes
///
/// Implementations should:
/// - Be async-safe for concurrent job updates
/// - Handle serialization/deserialization of JobInfo
/// - Provide atomic writes where possible
/// - Log errors but not fail the in-memory operation
///
/// # Note on Trait Design
///
/// This trait uses `async_trait` for compatibility with trait objects (dyn JobPersistence).
/// While native async traits are preferred for internal traits, public library traits
/// that need dynamic dispatch require async_trait to be dyn-compatible.
#[async_trait]
pub trait JobPersistence: Send + Sync {
    /// Persist job metadata update
    ///
    /// Called whenever job status, progress, heartbeat, or retry info changes.
    /// The implementation should serialize and store the JobInfo to durable storage.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Unique job identifier
    /// * `job_info` - Current job metadata snapshot
    ///
    /// # Errors
    ///
    /// Returns an error if the persistence operation fails. The caller should log
    /// but not fail the in-memory update.
    async fn persist_job(&self, job_id: &JobId, job_info: &JobInfo) -> Result<()>;

    /// Delete job metadata
    ///
    /// Called when explicitly removing a job from the system (e.g., cleanup operations).
    /// The implementation should remove both metadata and associated context data.
    ///
    /// # Arguments
    ///
    /// * `job_id` - Unique job identifier
    ///
    /// # Errors
    ///
    /// Returns an error if the deletion fails.
    async fn delete_job(&self, job_id: &JobId) -> Result<()>;
}
