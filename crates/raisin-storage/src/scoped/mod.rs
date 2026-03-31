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
