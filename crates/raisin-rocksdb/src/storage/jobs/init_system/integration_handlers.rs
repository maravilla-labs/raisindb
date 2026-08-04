//! Integration (virtual-node) job handler construction.
//!
//! Mirrors the sibling `*_handlers` modules: builds the handlers that back the
//! outbound-integration job types — the OAuth token-refresh handler and the
//! virtual-mount sync engine.

use std::sync::Arc;

use raisin_locks::LockManagerHandle;

use crate::jobs::handlers::virtual_mount_sync::{AdapterInvokerHandle, FunctionAdapterInvoker};
use crate::jobs::{
    FunctionExecutorCallback, IntegrationTokenRefreshHandler, VirtualMountSyncHandler,
};
use crate::storage::RocksDBStorage;

/// Create the integration OAuth token-refresh handler.
///
/// `lock_manager` serializes each integration's refresh across the cluster.
/// When `None`, refresh is single-node only: the periodic driver's dedup key
/// does NOT prevent every node running its own sweep, so two nodes can present
/// the same rotating refresh token concurrently and disconnect the account.
pub fn create_integration_token_refresh_handler(
    storage: Arc<RocksDBStorage>,
    lock_manager: Option<LockManagerHandle>,
) -> Arc<IntegrationTokenRefreshHandler> {
    Arc::new(IntegrationTokenRefreshHandler::new(storage, lock_manager))
}

/// Create the virtual-mount sync engine handler.
///
/// `function_executor` backs the adapter invoker (dependency inversion — the
/// engine invokes adapter functions directly, never as a nested job). When
/// `None`, sync jobs no-op with a warning. `lock_manager` enables cluster-safe
/// per-mount lease locking; when `None`, the engine uses dedup-key-only
/// semantics.
pub fn create_virtual_mount_sync_handler(
    storage: Arc<RocksDBStorage>,
    function_executor: Option<FunctionExecutorCallback>,
    lock_manager: Option<LockManagerHandle>,
) -> Arc<VirtualMountSyncHandler> {
    let invoker: Option<AdapterInvokerHandle> = function_executor
        .map(|cb| Arc::new(FunctionAdapterInvoker::new(cb)) as AdapterInvokerHandle);
    Arc::new(VirtualMountSyncHandler::new(storage, invoker, lock_manager))
}
