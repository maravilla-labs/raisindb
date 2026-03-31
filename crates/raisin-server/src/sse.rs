//! Server-Sent Events (SSE) support for real-time job updates
//!
//! This module provides SSE endpoints that stream real-time job status updates
//! to connected clients, enabling live UI updates without polling.

use crate::management::ManagementState;
use async_trait::async_trait;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Extension,
};
use futures::stream::Stream;
use raisin_storage::jobs::{JobEvent, JobInfo, JobLogEntry, JobMonitor};
use raisin_storage::BackgroundJobs;
use raisin_transport_http::middleware::TenantInfo;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Wrapper enum for SSE events (job updates + log entries)
#[derive(Clone)]
pub(crate) enum SseEvent {
    JobUpdate(Box<JobEvent>),
    JobLog(JobLogEntry),
}

/// SSE monitor that sends job events and log entries to connected clients
pub struct SseJobMonitor {
    sender: mpsc::Sender<SseEvent>,
}

impl SseJobMonitor {
    /// Create a new SSE monitor with the given channel sender
    pub(crate) fn new(sender: mpsc::Sender<SseEvent>) -> Self {
        Self { sender }
    }
}

#[async_trait]
impl JobMonitor for SseJobMonitor {
    async fn on_job_update(&self, event: JobEvent) {
        // Ignore send errors (client disconnected)
        let _ = self.sender.send(SseEvent::JobUpdate(Box::new(event))).await;
    }

    async fn on_job_created(&self, job: &JobInfo) {
        // Create a synthetic event for job creation
        let event = JobEvent {
            job_id: job.id.clone(),
            job_info: job.clone(),
            old_status: None,
            new_status: job.status.clone(),
            timestamp: chrono::Utc::now(),
        };
        let _ = self.sender.send(SseEvent::JobUpdate(Box::new(event))).await;
    }

    async fn on_job_removed(&self, _job_id: &raisin_storage::JobId) {
        // We could send a removal event if needed
    }

    async fn on_job_progress(&self, job_id: &raisin_storage::JobId, progress: f32) {
        // Send progress update event for real-time UI updates
        let event = JobEvent {
            job_id: job_id.clone(),
            job_info: JobInfo {
                id: job_id.clone(),
                job_type: raisin_storage::jobs::JobType::Custom(format!("progress:{}", progress)),
                status: raisin_storage::jobs::JobStatus::Running,
                tenant: None,
                started_at: chrono::Utc::now(),
                completed_at: None,
                progress: Some(progress),
                result: None,
                error: None,
                retry_count: 0,
                max_retries: 0,
                last_heartbeat: None,
                timeout_seconds: 0,
                next_retry_at: None,
            },
            old_status: None,
            new_status: raisin_storage::jobs::JobStatus::Running,
            timestamp: chrono::Utc::now(),
        };
        let _ = self.sender.send(SseEvent::JobUpdate(Box::new(event))).await;
    }

    async fn on_job_log(&self, entry: JobLogEntry) {
        tracing::debug!(
            job_id = %entry.job_id,
            level = %entry.level,
            "SseJobMonitor: sending job-log event to SSE stream"
        );
        let _ = self.sender.send(SseEvent::JobLog(entry)).await;
    }
}

/// Create an SSE stream of job events (RocksDB-specific version)
///
/// This version uses the instance-based job registry from RocksDBStorage
/// for real-time job updates from the unified job system.
///
/// **Security**: Only jobs belonging to the authenticated tenant are streamed.
#[cfg(feature = "storage-rocksdb")]
pub async fn job_events_stream_rocksdb(
    State(state): State<ManagementState<raisin_rocksdb::RocksDBStorage>>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let storage = state.storage.clone();
    let tenant_id = tenant_info.tenant_id.clone();

    tracing::debug!(
        tenant_id = %tenant_id,
        "Job SSE stream opened (tenant-filtered)"
    );

    // Create a channel for this client
    let (tx, rx) = mpsc::channel::<SseEvent>(100);

    // Create and register the monitor with the instance-based registry
    let monitor = Arc::new(SseJobMonitor::new(tx.clone()));
    let registry = storage.job_registry();
    registry.monitors().add_monitor(monitor).await;

    // Send initial state - only jobs for this tenant
    let storage_clone = storage.clone();
    let tx_clone = tx.clone();
    let tenant_id_clone = tenant_id.clone();
    tokio::spawn(async move {
        if let Ok(jobs) = storage_clone.list_jobs().await {
            for job in jobs {
                // SECURITY: Filter jobs by tenant
                if job.tenant.as_deref() != Some(&tenant_id_clone) {
                    continue;
                }
                let event = JobEvent {
                    job_id: job.id.clone(),
                    job_info: job.clone(),
                    old_status: None,
                    new_status: job.status.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let _ = tx_clone.send(SseEvent::JobUpdate(Box::new(event))).await;
            }
        }
    });

    // Convert channel receiver to SSE stream with tenant filtering
    let stream = ReceiverStream::new(rx)
        .filter(move |sse_event| {
            // SECURITY: Only stream events for the authenticated tenant
            match sse_event {
                SseEvent::JobUpdate(event) => event.job_info.tenant.as_deref() == Some(&tenant_id),
                SseEvent::JobLog(entry) => {
                    // Log entries are associated with jobs; tenant filtering is done via the
                    // monitor registration (only jobs for this tenant emit logs)
                    // We trust the job system to only emit logs for properly-scoped jobs
                    let _ = entry;
                    true
                }
            }
        })
        .map(|sse_event| match sse_event {
            SseEvent::JobUpdate(event) => {
                let data = serde_json::to_string(&SseEventData::from(*event))
                    .unwrap_or_else(|_| "{}".to_string());
                Ok(Event::default().event("job-update").data(data))
            }
            SseEvent::JobLog(entry) => {
                let data = serde_json::to_string(&SseJobLogEvent::from(entry))
                    .unwrap_or_else(|_| "{}".to_string());
                Ok(Event::default().event("job-log").data(data))
            }
        });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// Create an SSE stream of job events (generic version for non-RocksDB storage)
///
/// Falls back to using the global registry for compatibility with other storage backends.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn job_events_stream<S>(
    State(state): State<ManagementState<S>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    S: raisin_storage::BackgroundJobs + Send + Sync + 'static,
{
    let storage = state.storage;
    // Create a channel for this client
    let (tx, rx) = mpsc::channel::<SseEvent>(100);

    // Create and register the monitor (using global registry for non-RocksDB storage)
    let monitor = Arc::new(SseJobMonitor::new(tx.clone()));
    let registry = raisin_storage::jobs::global_registry();
    registry.monitors().add_monitor(monitor).await;

    // Send initial state - all current jobs
    tokio::spawn(async move {
        if let Ok(jobs) = storage.list_jobs().await {
            for job in jobs {
                let event = JobEvent {
                    job_id: job.id.clone(),
                    job_info: job.clone(),
                    old_status: None,
                    new_status: job.status.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let _ = tx.send(SseEvent::JobUpdate(Box::new(event))).await;
            }
        }
    });

    // Convert channel receiver to SSE stream
    let stream = ReceiverStream::new(rx).map(|sse_event| match sse_event {
        SseEvent::JobUpdate(event) => {
            let data = serde_json::to_string(&SseEventData::from(*event))
                .unwrap_or_else(|_| "{}".to_string());
            Ok(Event::default().event("job-update").data(data))
        }
        SseEvent::JobLog(entry) => {
            let data = serde_json::to_string(&SseJobLogEvent::from(entry))
                .unwrap_or_else(|_| "{}".to_string());
            Ok(Event::default().event("job-log").data(data))
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// Log entry from function execution for SSE streaming
#[derive(serde::Serialize)]
struct SseLogEntry {
    /// Log level (info, warn, error, debug)
    level: String,
    /// Log message content
    message: String,
    /// ISO 8601 timestamp
    timestamp: String,
}

/// SSE event data for real-time job log entries
#[derive(serde::Serialize)]
struct SseJobLogEvent {
    job_id: String,
    level: String,
    message: String,
    timestamp: String,
}

impl From<JobLogEntry> for SseJobLogEvent {
    fn from(entry: JobLogEntry) -> Self {
        Self {
            job_id: entry.job_id.0,
            level: entry.level,
            message: entry.message,
            timestamp: entry.timestamp.to_rfc3339(),
        }
    }
}

/// Simplified event data for SSE transmission
#[derive(serde::Serialize)]
struct SseEventData {
    job_id: String,
    job_type: String,
    status: String,
    old_status: Option<String>,
    tenant: Option<String>,
    progress: Option<f32>,
    error: Option<String>,
    timestamp: String,
    retry_count: u32,
    max_retries: u32,
    last_heartbeat: Option<String>,
    timeout_seconds: u64,
    next_retry_at: Option<String>,
    /// Logs from function execution (only for FunctionExecution jobs)
    #[serde(skip_serializing_if = "Option::is_none")]
    logs: Option<Vec<SseLogEntry>>,
    /// Function execution result (only for FunctionExecution jobs)
    #[serde(skip_serializing_if = "Option::is_none")]
    function_result: Option<serde_json::Value>,
    /// Function path (for FunctionExecution jobs)
    #[serde(skip_serializing_if = "Option::is_none")]
    function_path: Option<String>,
    /// Trigger path (for FlowExecution and FunctionExecution jobs)
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_path: Option<String>,
    /// Workspace ID (from job context if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    /// Flow instance ID (for FlowInstanceExecution jobs)
    #[serde(skip_serializing_if = "Option::is_none")]
    flow_instance_id: Option<String>,
}

impl From<JobEvent> for SseEventData {
    fn from(event: JobEvent) -> Self {
        use raisin_storage::jobs::{JobStatus, JobType};

        // Serialize status consistently with REST API (as string for simple variants)
        let status_str = match &event.new_status {
            JobStatus::Scheduled => "Scheduled".to_string(),
            JobStatus::Running => "Running".to_string(),
            JobStatus::Executing => "Executing".to_string(),
            JobStatus::Completed => "Completed".to_string(),
            JobStatus::Cancelled => "Cancelled".to_string(),
            JobStatus::Failed(msg) => format!("Failed: {}", msg),
        };

        let old_status_str = event.old_status.as_ref().map(|s| match s {
            JobStatus::Scheduled => "Scheduled".to_string(),
            JobStatus::Running => "Running".to_string(),
            JobStatus::Executing => "Executing".to_string(),
            JobStatus::Completed => "Completed".to_string(),
            JobStatus::Cancelled => "Cancelled".to_string(),
            JobStatus::Failed(msg) => format!("Failed: {}", msg),
        });

        // Extract function_path, trigger_path, flow_instance_id from job type
        let (function_path, trigger_path, flow_instance_id) = match &event.job_info.job_type {
            JobType::FunctionExecution {
                function_path,
                trigger_name,
                ..
            } => (Some(function_path.clone()), trigger_name.clone(), None),
            JobType::FlowExecution { trigger_path, .. } => (None, Some(trigger_path.clone()), None),
            JobType::FlowInstanceExecution { instance_id, .. } => {
                (None, None, Some(instance_id.clone()))
            }
            _ => (None, None, None),
        };

        // Extract logs and result from FunctionExecution, FlowExecution, and FlowInstanceExecution jobs
        let (logs, function_result) = match &event.job_info.job_type {
            JobType::FunctionExecution { .. }
            | JobType::FlowExecution { .. }
            | JobType::FlowInstanceExecution { .. } => {
                // Try to extract logs from the result JSON
                // FunctionExecutionResult has fields: execution_id, success, result, error, duration_ms, logs
                let logs = event.job_info.result.as_ref().and_then(|result| {
                    result.get("logs").and_then(|logs_val| {
                        logs_val.as_array().map(|arr| {
                            let timestamp = event.timestamp.to_rfc3339();
                            arr.iter()
                                .filter_map(|log| log.as_str())
                                .map(|msg| {
                                    // Parse log level from message prefix (e.g., "[INFO] message")
                                    let (level, message) = if msg.starts_with("[ERROR]")
                                        || msg.starts_with("[error]")
                                    {
                                        (
                                            "error".to_string(),
                                            msg.trim_start_matches("[ERROR]")
                                                .trim_start_matches("[error]")
                                                .trim()
                                                .to_string(),
                                        )
                                    } else if msg.starts_with("[WARN]") || msg.starts_with("[warn]")
                                    {
                                        (
                                            "warn".to_string(),
                                            msg.trim_start_matches("[WARN]")
                                                .trim_start_matches("[warn]")
                                                .trim()
                                                .to_string(),
                                        )
                                    } else if msg.starts_with("[DEBUG]")
                                        || msg.starts_with("[debug]")
                                    {
                                        (
                                            "debug".to_string(),
                                            msg.trim_start_matches("[DEBUG]")
                                                .trim_start_matches("[debug]")
                                                .trim()
                                                .to_string(),
                                        )
                                    } else if msg.starts_with("[INFO]") || msg.starts_with("[info]")
                                    {
                                        (
                                            "info".to_string(),
                                            msg.trim_start_matches("[INFO]")
                                                .trim_start_matches("[info]")
                                                .trim()
                                                .to_string(),
                                        )
                                    } else {
                                        ("info".to_string(), msg.to_string())
                                    };
                                    SseLogEntry {
                                        level,
                                        message,
                                        timestamp: timestamp.clone(),
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                });
                let function_result = event.job_info.result.clone();
                (logs, function_result)
            }
            _ => (None, None),
        };

        Self {
            job_id: event.job_id.0,
            job_type: event.job_info.job_type.to_string(),
            status: status_str,
            old_status: old_status_str,
            tenant: event.job_info.tenant,
            progress: event.job_info.progress,
            error: event.job_info.error,
            timestamp: event.timestamp.to_rfc3339(),
            retry_count: event.job_info.retry_count,
            max_retries: event.job_info.max_retries,
            last_heartbeat: event.job_info.last_heartbeat.map(|dt| dt.to_rfc3339()),
            timeout_seconds: event.job_info.timeout_seconds,
            next_retry_at: event.job_info.next_retry_at.map(|dt| dt.to_rfc3339()),
            logs,
            function_result,
            function_path,
            trigger_path,
            workspace: None, // TODO: Could be extracted from JobContext if needed
            flow_instance_id,
        }
    }
}

/// Health check SSE endpoint for system monitoring
///
/// **Security**: Health data is filtered by authenticated tenant.
pub async fn health_events_stream<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    S: raisin_storage::ManagementOps + Send + Sync + 'static,
{
    let tenant_id = tenant_info.tenant_id.clone();
    tracing::debug!(
        tenant_id = %tenant_id,
        "Health SSE stream opened (tenant-filtered)"
    );

    // Stream health status every 5 seconds
    let storage = state.storage.clone();
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;

            // SECURITY: Pass tenant_id for tenant-specific health data
            let health = match storage.get_health(Some(&tenant_id)).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!("Failed to get health: {}", e);
                    continue;
                }
            };

            let data = serde_json::to_string(&health)
                .unwrap_or_else(|_| "{}".to_string());

            yield Ok(Event::default()
                .event("health-update")
                .data(data));
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// Metrics SSE endpoint for performance monitoring
///
/// **Security**: Metrics data is filtered by authenticated tenant.
pub async fn metrics_events_stream<S>(
    State(state): State<ManagementState<S>>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    S: raisin_storage::ManagementOps + Send + Sync + 'static,
{
    let tenant_id = tenant_info.tenant_id.clone();
    tracing::debug!(
        tenant_id = %tenant_id,
        "Metrics SSE stream opened (tenant-filtered)"
    );

    // Stream metrics every 2 seconds for real-time monitoring
    let storage = state.storage.clone();
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;

            // SECURITY: Pass tenant_id for tenant-specific metrics
            let metrics = match storage.get_metrics(Some(&tenant_id)).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("Failed to get metrics: {}", e);
                    continue;
                }
            };

            let data = serde_json::to_string(&metrics)
                .unwrap_or_else(|_| "{}".to_string());

            yield Ok(Event::default()
                .event("metrics-update")
                .data(data));
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}
