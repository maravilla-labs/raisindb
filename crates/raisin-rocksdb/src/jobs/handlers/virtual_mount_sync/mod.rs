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

mod content;
mod ctx;
mod finalize;
mod item;
mod misconfig;
mod phases;
mod preflight;
mod push;
mod resolve;
mod run;
mod state_store;
pub(crate) mod write;

mod adapter;
mod batch;
pub mod check;
mod config;
mod delta;
mod ephemeral;
mod full;
mod full_reconcile;
mod materializer;
mod registry_backfill;
mod subscription;

#[cfg(test)]
mod tests;

pub use adapter::{
    build_input, build_mount_snapshot, AdapterError, AdapterInvoker, AdapterInvokerHandle,
    Capabilities, FunctionAdapterInvoker,
};
pub use content::{ContentFetch, ContentTarget};
// Credential assembly lives in raisin-models so the sync engine and the HTTP
// connection-test handler cannot drift apart.
pub use batch::SyncBatcher;
pub use config::{
    child_external_id, default_mapping, excluded, mount_relative, passes_filters, Change,
    ChangesPage, ConnectedAccount, DrainSummary, ExternalItem, IntegrationConfig, ListPage,
    MappedChild, MappedItem, MappedNode, MountConfig, MountState, PathFilter, SyncConfig, SyncRun,
    WriteConfig, CHILD_ID_SEP, MAX_RUN_HISTORY, SYNC_ACTOR, SYSTEM_WORKSPACE,
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
use ctx::{SYNC_LEASE_TTL, SYNC_WALL_CLOCK_BUDGET};
pub use item::{
    map_item, map_to_external, stage_item, MapperWriteback, ToExternal, ToExternalOutcome,
};
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
    /// Where fetched attachment bytes go. `None` on a deployment with no binary
    /// storage wired: the sync still materializes attachment METADATA, and only
    /// [`Self::fetch_content`] fails — the read path must not depend on a
    /// capability only the on-demand fetch needs.
    binary_store: Option<crate::jobs::handlers::package_install::BinaryStorageCallback>,
    /// Where a locally-authored node's file bytes are READ from, for the write
    /// path: a `raisin:Asset` created under a mirror mount is metadata plus a
    /// storage key, and the provider wants the bytes.
    ///
    /// The mirror image of `binary_store`, and optional for the same reason —
    /// a deployment without it still syncs metadata in both directions, and
    /// only a content-carrying push fails, with a reason that says so.
    binary_retrieval: Option<crate::jobs::handlers::package_install::BinaryRetrievalCallback>,
    /// How a cached mount file is DELETED once its TTL passes.
    ///
    /// Optional like its siblings: without it the content TTL still clears the
    /// node's `file`, and only the object is left behind — a disk cost, not a
    /// correctness one, and visible in the log.
    binary_delete: Option<crate::jobs::handlers::package_install::BinaryDeleteCallback>,
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
            binary_store: None,
            binary_delete: None,
            binary_retrieval: None,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Wire the binary store the on-demand attachment fetch writes into.
    ///
    /// Optional and separate from [`Self::new`] because the callback lives in
    /// the transport layer, which is built after the job handlers; a sync
    /// engine with no binary store is fully functional for everything except
    /// [`Self::fetch_content`].
    pub fn with_binary_store(
        mut self,
        callback: crate::jobs::handlers::package_install::BinaryStorageCallback,
    ) -> Self {
        self.binary_store = Some(callback);
        self
    }

    /// Wire the binary retrieval the WRITE path reads node bytes through.
    ///
    /// Separate from [`Self::with_binary_store`] because they are opposite
    /// directions and a deployment can legitimately have one and not the other:
    /// the store is what an inbound attachment is written into, this is what an
    /// outbound file is read out of.
    pub fn with_binary_retrieval(
        mut self,
        callback: crate::jobs::handlers::package_install::BinaryRetrievalCallback,
    ) -> Self {
        self.binary_retrieval = Some(callback);
        self
    }

    /// Wire the deletion the content TTL drops expired cached files through.
    pub fn with_binary_delete(
        mut self,
        callback: crate::jobs::handlers::package_install::BinaryDeleteCallback,
    ) -> Self {
        self.binary_delete = Some(callback);
        self
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
            // The latency drain is an ordinary run in `mode: "write"`, NOT a
            // second execution path. It reuses preflight, the lease, the
            // capabilities probe, the credential, the batcher and the drain
            // itself, so a drain triggered by an edit and the drain that runs
            // inside a sync cannot drift apart. `run_phases` stops after the
            // drain in this mode: nothing is read from the provider.
            JobType::VirtualMountWriteDrain { mount_id, trigger } => {
                self.run_sync(
                    job,
                    context,
                    mount_id.clone(),
                    write::WRITE_ONLY_MODE.to_string(),
                    trigger.clone(),
                )
                .await
            }
            JobType::VirtualMountWriteReconcile { tenant_id, repo_id } => {
                write::run_write_reconcile(
                    &self.storage,
                    self.lock_manager.as_ref(),
                    &self.instance_id,
                    tenant_id.clone(),
                    repo_id.clone(),
                )
                .await
                .map(|_| None)
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
