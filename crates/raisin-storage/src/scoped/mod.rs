//! Scoped storage wrapper for multi-tenancy
//!
//! This module provides a wrapper around any Storage implementation
//! that automatically applies tenant context to all operations.

use crate::{IsolationMode, Storage, TenantContext};
use raisin_error::Result;
use std::marker::PhantomData;
use std::sync::Arc;

/// A scoped storage that wraps another storage with tenant context
///
/// This allows you to create tenant-isolated views of the same underlying
/// storage without modifying the storage implementation itself.
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_storage::{ScopedStorage, TenantContext};
/// use raisin_storage_rocks::RocksStorage;
///
/// let storage = Arc::new(RocksStorage::open("./data").unwrap());
/// let ctx = TenantContext::new("tenant-123", "production");
///
/// // Create a scoped view for this tenant
/// let scoped = ScopedStorage::new(storage, ctx);
///
/// // All operations now automatically include tenant context
/// scoped.nodes().get("workspace", "node-id").await;
/// ```
pub struct ScopedStorage<S: Storage> {
    inner: Arc<S>,
    context: TenantContext,
    _phantom: PhantomData<S>,
}

impl<S: Storage> ScopedStorage<S> {
    /// Create a new scoped storage with the given tenant context
    pub fn new(storage: Arc<S>, context: TenantContext) -> Self {
        Self {
            inner: storage,
            context,
            _phantom: PhantomData,
        }
    }

    /// Get the tenant context for this scoped storage
    pub fn context(&self) -> &TenantContext {
        &self.context
    }

    /// Get the underlying storage
    pub fn inner(&self) -> &Arc<S> {
        &self.inner
    }
}

// For now, ScopedStorage just delegates to the underlying storage.
// In the future, storage implementations can check for tenant context
// and apply appropriate prefixing/filtering.
impl<S: Storage> Storage for ScopedStorage<S> {
    type Tx = S::Tx;
    type Nodes = S::Nodes;
    type NodeTypes = S::NodeTypes;
    type Workspaces = S::Workspaces;
    type Registry = S::Registry;
    type PropertyIndex = S::PropertyIndex;
    type ReferenceIndex = S::ReferenceIndex;
    type Versioning = S::Versioning;

    fn nodes(&self) -> &Self::Nodes {
        self.inner.nodes()
    }

    fn node_types(&self) -> &Self::NodeTypes {
        self.inner.node_types()
    }

    fn workspaces(&self) -> &Self::Workspaces {
        self.inner.workspaces()
    }

    fn registry(&self) -> &Self::Registry {
        self.inner.registry()
    }

    fn property_index(&self) -> &Self::PropertyIndex {
        self.inner.property_index()
    }

    fn reference_index(&self) -> &Self::ReferenceIndex {
        self.inner.reference_index()
    }

    fn versioning(&self) -> &Self::Versioning {
        self.inner.versioning()
    }

    async fn begin(&self) -> Result<Self::Tx> {
        self.inner.begin().await
    }
}

/// BackgroundJobs forwards to the inner storage, injecting tenant from the scope context.
///
/// The trait-level `tenant: &str` arg is ignored — callers using the trait method on a
/// `ScopedStorage` get the scope's tenant. Prefer the `*_scoped()` helpers below for
/// call sites where the lack of a tenant arg makes the intent clearer.
#[async_trait::async_trait]
impl<S> crate::BackgroundJobs for ScopedStorage<S>
where
    S: crate::BackgroundJobs + Storage + Send + Sync,
{
    fn start_background_jobs(&self) -> Result<crate::JobHandle> {
        self.inner.start_background_jobs()
    }

    fn schedule_integrity_scan(
        &self,
        _tenant: &str,
        interval: std::time::Duration,
    ) -> Result<crate::JobId> {
        self.inner
            .schedule_integrity_scan(self.context.tenant_id(), interval)
    }

    async fn get_job_status(
        &self,
        _tenant: &str,
        job_id: &crate::JobId,
    ) -> Result<crate::JobStatus> {
        self.inner
            .get_job_status(self.context.tenant_id(), job_id)
            .await
    }

    async fn cancel_job(&self, _tenant: &str, job_id: &crate::JobId) -> Result<()> {
        self.inner
            .cancel_job(self.context.tenant_id(), job_id)
            .await
    }

    async fn delete_job(&self, _tenant: &str, job_id: &crate::JobId) -> Result<()> {
        self.inner
            .delete_job(self.context.tenant_id(), job_id)
            .await
    }

    async fn delete_jobs_batch(
        &self,
        _tenant: &str,
        job_ids: &[crate::JobId],
    ) -> (usize, usize) {
        self.inner
            .delete_jobs_batch(self.context.tenant_id(), job_ids)
            .await
    }

    async fn list_jobs(&self, _tenant: &str) -> Result<Vec<crate::JobInfo>> {
        self.inner.list_jobs(self.context.tenant_id()).await
    }

    async fn wait_for_job(
        &self,
        _tenant: &str,
        job_id: &crate::JobId,
    ) -> Result<crate::JobStatus> {
        self.inner
            .wait_for_job(self.context.tenant_id(), job_id)
            .await
    }

    async fn get_job_info(
        &self,
        _tenant: &str,
        job_id: &crate::JobId,
    ) -> Result<crate::JobInfo> {
        self.inner
            .get_job_info(self.context.tenant_id(), job_id)
            .await
    }

    async fn purge_all_jobs(&self, _tenant: &str) -> Result<usize> {
        self.inner.purge_all_jobs(self.context.tenant_id()).await
    }

    async fn purge_orphaned_jobs(&self, _tenant: &str) -> Result<usize> {
        self.inner
            .purge_orphaned_jobs(self.context.tenant_id())
            .await
    }

    async fn get_job_queue_stats(&self, _tenant: &str) -> Result<crate::JobQueueStats> {
        self.inner
            .get_job_queue_stats(self.context.tenant_id())
            .await
    }

    async fn force_fail_stuck_jobs(
        &self,
        _tenant: &str,
        stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)> {
        self.inner
            .force_fail_stuck_jobs(self.context.tenant_id(), stuck_minutes)
            .await
    }
}

/// Convenience helpers that don't take an explicit tenant arg — they use the
/// `ScopedStorage`'s context. Prefer these in server handlers built from a
/// per-request `ScopedStorage`.
impl<S> ScopedStorage<S>
where
    S: crate::BackgroundJobs + Storage + Send + Sync,
{
    pub async fn list_jobs_scoped(&self) -> Result<Vec<crate::JobInfo>> {
        self.inner.list_jobs(self.context.tenant_id()).await
    }

    pub async fn get_job_info_scoped(
        &self,
        job_id: &crate::JobId,
    ) -> Result<crate::JobInfo> {
        self.inner
            .get_job_info(self.context.tenant_id(), job_id)
            .await
    }

    pub async fn get_job_status_scoped(
        &self,
        job_id: &crate::JobId,
    ) -> Result<crate::JobStatus> {
        self.inner
            .get_job_status(self.context.tenant_id(), job_id)
            .await
    }

    pub async fn cancel_job_scoped(&self, job_id: &crate::JobId) -> Result<()> {
        self.inner
            .cancel_job(self.context.tenant_id(), job_id)
            .await
    }

    pub async fn delete_job_scoped(&self, job_id: &crate::JobId) -> Result<()> {
        self.inner
            .delete_job(self.context.tenant_id(), job_id)
            .await
    }

    pub async fn delete_jobs_batch_scoped(
        &self,
        job_ids: &[crate::JobId],
    ) -> (usize, usize) {
        self.inner
            .delete_jobs_batch(self.context.tenant_id(), job_ids)
            .await
    }

    pub async fn wait_for_job_scoped(
        &self,
        job_id: &crate::JobId,
    ) -> Result<crate::JobStatus> {
        self.inner
            .wait_for_job(self.context.tenant_id(), job_id)
            .await
    }

    pub async fn purge_all_jobs_scoped(&self) -> Result<usize> {
        self.inner.purge_all_jobs(self.context.tenant_id()).await
    }

    pub async fn purge_orphaned_jobs_scoped(&self) -> Result<usize> {
        self.inner
            .purge_orphaned_jobs(self.context.tenant_id())
            .await
    }

    pub async fn get_job_queue_stats_scoped(&self) -> Result<crate::JobQueueStats> {
        self.inner
            .get_job_queue_stats(self.context.tenant_id())
            .await
    }

    pub async fn force_fail_stuck_jobs_scoped(
        &self,
        stuck_minutes: u64,
    ) -> Result<(usize, Vec<String>)> {
        self.inner
            .force_fail_stuck_jobs(self.context.tenant_id(), stuck_minutes)
            .await
    }

    pub fn schedule_integrity_scan_scoped(
        &self,
        interval: std::time::Duration,
    ) -> Result<crate::JobId> {
        self.inner
            .schedule_integrity_scan(self.context.tenant_id(), interval)
    }
}

/// Trait for storage implementations that support native scoping
///
/// This is separate from StorageExt because some implementations (like RocksStorage)
/// can provide more efficient scoping by modifying their internal state rather than
/// wrapping in ScopedStorage.
pub trait ScopableStorage: Storage + Sized + Clone {
    /// Create a scoped version of this storage for a specific tenant
    ///
    /// Unlike StorageExt::scoped() which wraps the storage, this method may
    /// modify the storage's internal state for more efficient operation.
    fn scope(self, context: TenantContext) -> Self;
}

/// Extension trait to add scoping capabilities to any Storage
pub trait StorageExt: Storage + Sized {
    /// Create a scoped view of this storage for a specific tenant
    fn scoped(self: Arc<Self>, context: TenantContext) -> ScopedStorage<Self> {
        ScopedStorage::new(self, context)
    }

    /// Create a scoped view using isolation mode
    fn with_isolation(self: Arc<Self>, mode: IsolationMode) -> Option<ScopedStorage<Self>> {
        match mode {
            IsolationMode::Single => None,
            IsolationMode::Shared(ctx) => Some(ScopedStorage::new(self, ctx)),
            IsolationMode::Dedicated { context, .. } => {
                // For dedicated mode, the connection string should be used to create
                // a new storage instance, but we'll use the context for now
                Some(ScopedStorage::new(self, context))
            }
        }
    }
}

// Implement for all Storage types
impl<S: Storage> StorageExt for S {}

#[cfg(test)]
mod tests;
