//! Virtual-mount sync engine.
//!
//! The core of the Virtual Nodes feature: keeps external systems mounted into
//! workspaces in sync by invoking adapter functions and materializing their
//! results through the normal transactional write path.
//!
//! Handles two job types:
//! - [`JobType::VirtualMountSyncCheck`] — periodic scan enqueuing due syncs.
//! - [`JobType::VirtualMountSync`] — sync one mount (delta or full).
//!
//! `raisin-rocksdb` cannot depend on `raisin-functions`, so adapters are invoked
//! through an injected [`FunctionExecutorCallback`] (dependency inversion). A
//! per-mount lease lock from `raisin-locks` (when configured) makes syncs
//! cluster-safe; without it the engine falls back to dedup-key-only semantics.

mod ctx;
mod finalize;
mod item;
mod preflight;
mod push;
mod resolve;
mod run;
mod state_store;

mod adapter;
mod batch;
pub mod check;
mod config;
mod delta;
mod ephemeral;
mod full;
mod full_reconcile;
mod materializer;
mod subscription;

#[cfg(test)]
mod tests;

pub use adapter::{
    build_input, build_mount_snapshot, AdapterError, AdapterInvoker, AdapterInvokerHandle,
    Capabilities, FunctionAdapterInvoker,
};
// Credential assembly lives in raisin-models so the sync engine and the HTTP
// connection-test handler cannot drift apart.
pub use batch::SyncBatcher;
pub use config::{
    default_mapping, passes_filters, Change, ChangesPage, ConnectedAccount, ExternalItem,
    IntegrationConfig, ListPage, MappedNode, MountConfig, MountState, SyncConfig, SyncRun,
    WriteConfig, MAX_RUN_HISTORY, SYNC_ACTOR, SYSTEM_WORKSPACE,
};
pub use materializer::{
    BatchOp, BatchStats, MountScope, NodeMaterializer, RocksDbMaterializer, SyncIndex, VirtualMeta,
    VirtualNodeRef,
};
pub use raisin_models::nodes::integrations::build_credential;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_error::{Error, Result};
use raisin_locks::LockManagerHandle;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::integrations::AccountSelectionError;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{BranchRepository, RepositoryManagementRepository, Storage};
use serde_json::{json, Value};

use crate::RocksDBStorage;
pub use ctx::SyncCtx;
use ctx::SYNC_LEASE_TTL;
pub use item::{map_item, stage_item};
use preflight::{skip_result, Preflight};
use resolve::CtxParts;
pub use state_store::{
    persist_mount_state, read_mount_state, record_push_delivery, stop_requested,
};
use state_store::{persist_state, StateWrite};

/// Handler for the two virtual-mount job types.
pub struct VirtualMountSyncHandler {
    storage: Arc<RocksDBStorage>,
    invoker: Option<AdapterInvokerHandle>,
    materializer: Arc<dyn NodeMaterializer>,
    lock_manager: Option<LockManagerHandle>,
    /// Per-process identity, used to build the lease owner string.
    instance_id: String,
}

impl VirtualMountSyncHandler {
    /// Create a handler. `invoker` is `None` when no function executor is wired
    /// (adapter calls then fail cleanly); `lock_manager` is `None` when the
    /// locks subsystem is disabled (single-node dedup semantics).
    pub fn new(
        storage: Arc<RocksDBStorage>,
        invoker: Option<AdapterInvokerHandle>,
        lock_manager: Option<LockManagerHandle>,
    ) -> Self {
        let materializer = Arc::new(RocksDbMaterializer::new(storage.clone()));
        Self {
            storage,
            invoker,
            materializer,
            lock_manager,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Dispatch a virtual-mount job.
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<Option<Value>> {
        match &job.job_type {
            JobType::VirtualMountSyncCheck { tenant_id, repo_id } => {
                check::run_check(&self.storage, tenant_id.clone(), repo_id.clone())
                    .await
                    .map(|_| None)
            }
            // The run summary is returned as the job result, not swallowed:
            // `JobInfo.result` was always null for mount syncs, so the only
            // record of what a sync did was an `info!` line that production's
            // `RUST_LOG=warn` discarded.
            JobType::VirtualMountSync {
                mount_id,
                mode,
                trigger,
            } => {
                self.run_sync(
                    job,
                    context,
                    mount_id.clone(),
                    mode.clone(),
                    trigger.clone(),
                )
                .await
            }
            JobType::VirtualMountSubscriptionRenew { tenant_id } => self
                .run_subscription_renew(tenant_id.clone())
                .await
                .map(|_| None),
            other => Err(Error::Validation(format!(
                "VirtualMountSyncHandler received unexpected job type: {other}"
            ))),
        }
    }
}
