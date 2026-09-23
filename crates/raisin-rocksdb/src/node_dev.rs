// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The node-development surface on RocksDB, cluster-safe with EXISTING
//! mechanisms only.
//!
//! [`raisin_core::services::node_dev`] serializes commits on a branch through
//! an `ExclusiveSections` provider. Here that is the flow runtime's pairing
//! (see [`crate::jobs::FlowInstanceLockManager`]): an in-process keyed mutex,
//! plus a `raisin-locks` lease whenever the locks subsystem is configured, so
//! two NODES never run a check-and-commit on one branch at once. The changes
//! and the changeset record are ordinary node writes, so they replicate the
//! way every node write does.
//!
//! The service is late-bound in a process slot, like the agent run host: the
//! HTTP routes and the function bindings read it at call time.

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use raisin_core::services::node_dev::{
    DevResult, ExclusiveSections, NodeDevError, NodeDevService, SectionGuard,
};
use raisin_locks::{FencingToken, LockManagerHandle};

use crate::jobs::keyed_mutex::{KeyedMutex, KeyedMutexGuard};
use crate::RocksDBStorage;

/// The node-development service over RocksDB.
pub type RocksNodeDev = NodeDevService<RocksDBStorage>;

/// Crash backstop only: a commit holds the section for milliseconds.
const LEASE_TTL: Duration = Duration::from_secs(30);
/// How long to try for the lease before answering `busy`.
const ACQUIRE_BUDGET: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(15);

/// Keyed mutex + optional distributed lease.
pub struct ClusterSections {
    local: Arc<KeyedMutex<String>>,
    distributed: Option<LockManagerHandle>,
    owner: String,
}

impl ClusterSections {
    /// Sections for this node; cluster-wide when `distributed` is set.
    pub fn new(distributed: Option<LockManagerHandle>, owner: impl Into<String>) -> Self {
        Self {
            local: Arc::new(KeyedMutex::new()),
            distributed,
            owner: owner.into(),
        }
    }
}

struct Held {
    lease: Option<(LockManagerHandle, String, FencingToken)>,
    _local: KeyedMutexGuard<String>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some((manager, key, token)) = self.lease.take() {
            // Idempotent and token-checked; worst case the lease expires.
            tokio::spawn(async move {
                let _ = manager.release(&key, token).await;
            });
        }
    }
}

#[async_trait]
impl ExclusiveSections for ClusterSections {
    async fn enter(&self, key: &str) -> DevResult<SectionGuard> {
        let local = self.local.lock(key.to_string()).await;
        let Some(manager) = &self.distributed else {
            return Ok(Box::new(Held {
                lease: None,
                _local: local,
            }));
        };
        let deadline = tokio::time::Instant::now() + ACQUIRE_BUDGET;
        loop {
            match manager.try_acquire(key, &self.owner, LEASE_TTL).await {
                Ok(Some(guard)) => {
                    return Ok(Box::new(Held {
                        lease: Some((manager.clone(), key.to_string(), guard.token)),
                        _local: local,
                    }))
                }
                Ok(None) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await
                }
                Ok(None) => {
                    return Err(NodeDevError::new(
                        503,
                        "busy",
                        "another node is committing to this branch; retry",
                    ))
                }
                Err(e) => {
                    // Same call the flow lock makes: keep working on the
                    // in-process guard and say so loudly.
                    tracing::warn!(key, error = %e, "node-dev lock backend unavailable - in-process serialization only");
                    return Ok(Box::new(Held {
                        lease: None,
                        _local: local,
                    }));
                }
            }
        }
    }
}

static SLOT: OnceLock<RwLock<Option<Arc<RocksNodeDev>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<RocksNodeDev>>> {
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Install this process's service.
pub fn install_node_dev(service: Arc<RocksNodeDev>) {
    *slot().write().expect("node-dev slot poisoned") = Some(service);
}

/// This process's service, once installed.
pub fn node_dev() -> Option<Arc<RocksNodeDev>> {
    slot().read().expect("node-dev slot poisoned").clone()
}

/// The installed service, or a local-only one over `storage` (embedded use,
/// tests, and a server that has not installed one yet).
pub fn node_dev_or_local(storage: &Arc<RocksDBStorage>) -> Arc<RocksNodeDev> {
    if let Some(s) = node_dev() {
        return s;
    }
    let mut w = slot().write().expect("node-dev slot poisoned");
    w.get_or_insert_with(|| Arc::new(NodeDevService::new(storage.clone())))
        .clone()
}

/// The node-development grant an agent run carries, if any:
/// `executor_config.node_dev.roots` on the run record. Read by the
/// transports from the RECORD, so a model can never widen it through its own
/// tool arguments.
pub async fn run_grant(
    tenant: &str,
    repo: &str,
    branch: &str,
    run_id: &str,
) -> Option<Vec<raisin_core::services::node_dev::WorkRoot>> {
    use raisin_agent_runtime::api::{self, Caller};
    use raisin_agent_runtime::ids::RunId;
    let host = crate::agent_runs::agent_run_host()?;
    let system = Caller {
        id: "system".into(),
        admin: true,
    };
    let view = api::get(
        &host,
        &api::scope(tenant, repo, Some(branch)),
        &RunId(run_id.to_string()),
        &system,
    )
    .await
    .ok()?;
    let roots = view
        .run
        .executor_config?
        .get("node_dev")?
        .get("roots")?
        .clone();
    serde_json::from_value(roots).ok()
}

/// The run id a tool call carries in `__raisin_context`, if any.
pub fn tool_run_id(args: &serde_json::Value) -> Option<String> {
    args.get("__raisin_context")?
        .get("run_id")?
        .as_str()
        .map(str::to_string)
}
