//! HTTP Management API endpoints for RaisinDB.
//!
//! Provides HTTP endpoints for management operations including:
//! - Health checks
//! - Integrity scanning
//! - Index rebuilding and verification
//! - Backup/restore
//! - Compaction and metrics
//! - Background job management

#[cfg(feature = "storage-rocksdb")]
pub mod admin;
mod backup;
pub mod dependencies;
#[cfg(feature = "storage-rocksdb")]
pub mod graph_cache;
mod health;
mod integrity;
mod jobs;
mod maintenance;
mod router;
mod types;

use std::sync::Arc;

/// Application state for management endpoints.
#[derive(Clone)]
pub struct ManagementState<S> {
    pub storage: Arc<S>,
    /// Process-wide shutdown signal, when the server installed one.
    ///
    /// `None` in every other construction (tests, embedded use), in which case
    /// nothing ever cancels and behaviour is unchanged. SSE handlers under this
    /// state MUST end their stream when it fires — an open stream is an open
    /// connection, and `axum::serve`'s graceful shutdown waits for those.
    pub shutdown: Option<tokio_util::sync::CancellationToken>,
}

impl<S> ManagementState<S> {
    /// A future that resolves when the server starts shutting down, or never
    /// when no token was installed. Pair with `StreamExt::take_until`.
    pub fn shutdown_signal(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        raisin_transport_http::shutdown_or_never(self.shutdown.clone())
    }
}

pub use router::management_router;
