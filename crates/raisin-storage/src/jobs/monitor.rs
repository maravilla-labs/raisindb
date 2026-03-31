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

//! Job monitoring system for real-time updates
//!
//! This module provides a pluggable monitoring system that allows
//! external systems to receive real-time updates about job status changes.
//!
//! Examples of monitors:
//! - SSE endpoint for web UI updates
//! - Redis publisher for distributed systems
//! - Webhook notifications
//! - Logging/metrics collectors

use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::types::{JobId, JobInfo, JobStatus};

/// A real-time log entry emitted by a running job
#[derive(Debug, Clone, Serialize)]
pub struct JobLogEntry {
    pub job_id: JobId,
    pub level: String,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Type alias for a log emitter callback
///
/// Called from function runtimes (QuickJS/Starlark) to stream logs in real-time.
/// Arguments: (level, message)
pub type LogEmitter = Arc<dyn Fn(String, String) + Send + Sync>;

/// Event emitted when a job's status changes
#[derive(Debug, Clone)]
pub struct JobEvent {
    pub job_id: JobId,
    pub job_info: JobInfo,
    pub old_status: Option<JobStatus>,
    pub new_status: JobStatus,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Trait for implementing job monitors
///
/// Monitors receive notifications when job status changes occur.
/// This allows for real-time updates to external systems.
#[async_trait]
pub trait JobMonitor: Send + Sync {
    /// Called when a job status changes
    async fn on_job_update(&self, event: JobEvent);

    /// Called when a new job is registered
    async fn on_job_created(&self, job: &JobInfo);

    /// Called when a job is removed from the registry
    async fn on_job_removed(&self, job_id: &JobId);

    /// Optional: Called periodically with progress updates
    async fn on_job_progress(&self, job_id: &JobId, progress: f32) {
        // Default implementation does nothing
        let _ = (job_id, progress);
    }

    /// Optional: Called when a job emits a log entry in real-time
    async fn on_job_log(&self, entry: JobLogEntry) {
        // Default implementation does nothing
        let _ = entry;
    }
}

/// Composite monitor that broadcasts events to multiple monitors
pub struct JobMonitorHub {
    monitors: Arc<RwLock<Vec<Arc<dyn JobMonitor>>>>,
}

impl JobMonitorHub {
    pub fn new() -> Self {
        Self {
            monitors: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a new monitor
    pub async fn add_monitor(&self, monitor: Arc<dyn JobMonitor>) {
        let mut monitors = self.monitors.write().await;
        monitors.push(monitor);
    }

    /// Remove all monitors
    pub async fn clear_monitors(&self) {
        let mut monitors = self.monitors.write().await;
        monitors.clear();
    }

    /// Broadcast an event to all registered monitors
    pub async fn broadcast_update(&self, event: JobEvent) {
        let monitors = self.monitors.read().await;
        for monitor in monitors.iter() {
            // Run each monitor notification in parallel
            let monitor = monitor.clone();
            let event = event.clone();
            tokio::spawn(async move {
                monitor.on_job_update(event).await;
            });
        }
    }

    /// Notify all monitors of a new job
    pub async fn broadcast_created(&self, job: &JobInfo) {
        let monitors = self.monitors.read().await;
        for monitor in monitors.iter() {
            let monitor = monitor.clone();
            let job = job.clone();
            tokio::spawn(async move {
                monitor.on_job_created(&job).await;
            });
        }
    }

    /// Notify all monitors of job removal
    pub async fn broadcast_removed(&self, job_id: &JobId) {
        let monitors = self.monitors.read().await;
        for monitor in monitors.iter() {
            let monitor = monitor.clone();
            let job_id = job_id.clone();
            tokio::spawn(async move {
                monitor.on_job_removed(&job_id).await;
            });
        }
    }

    /// Notify all monitors of job progress
    pub async fn broadcast_progress(&self, job_id: &JobId, progress: f32) {
        let monitors = self.monitors.read().await;
        for monitor in monitors.iter() {
            let monitor = monitor.clone();
            let job_id = job_id.clone();
            tokio::spawn(async move {
                monitor.on_job_progress(&job_id, progress).await;
            });
        }
    }

    /// Broadcast a real-time log entry to all monitors
    pub async fn broadcast_log(&self, entry: JobLogEntry) {
        let monitors = self.monitors.read().await;
        let monitor_count = monitors.len();
        tracing::debug!(
            job_id = %entry.job_id,
            level = %entry.level,
            monitor_count = monitor_count,
            "JobMonitorHub: broadcasting log to monitors"
        );
        for monitor in monitors.iter() {
            let monitor = monitor.clone();
            let entry = entry.clone();
            tokio::spawn(async move {
                monitor.on_job_log(entry).await;
            });
        }
    }
}

impl Default for JobMonitorHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Example monitor that logs all events (useful for debugging)
pub struct LoggingMonitor;

#[async_trait]
impl JobMonitor for LoggingMonitor {
    async fn on_job_update(&self, event: JobEvent) {
        tracing::info!(
            "Job {} status changed from {:?} to {:?}",
            event.job_id,
            event.old_status,
            event.new_status
        );
    }

    async fn on_job_created(&self, job: &JobInfo) {
        tracing::info!(
            "Job {} created: type={:?}, tenant={:?}",
            job.id,
            job.job_type,
            job.tenant
        );
    }

    async fn on_job_removed(&self, job_id: &JobId) {
        tracing::info!("Job {} removed from registry", job_id);
    }

    async fn on_job_progress(&self, job_id: &JobId, progress: f32) {
        tracing::debug!("Job {} progress: {:.1}%", job_id, progress * 100.0);
    }

    async fn on_job_log(&self, entry: JobLogEntry) {
        tracing::info!(
            job_id = %entry.job_id,
            level = %entry.level,
            "LoggingMonitor: job log: {}",
            entry.message
        );
    }
}
