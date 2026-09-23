// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The per-run (and per-subject) commit critical section.
//!
//! Same two layers as [`FlowInstanceLockManager`](crate::jobs::FlowInstanceLockManager),
//! for the same reason — a run is one aggregate several producers advance at
//! once (a driver finishing an operation, a stop arriving over HTTP on another
//! node, the sweeper taking over an expired lease):
//!
//! - an **in-process** keyed mutex, always present;
//! - the optional **distributed lease** from `raisin-locks`, whenever the locks
//!   subsystem is configured, so two NODES never commit to one run at once.
//!
//! Unlike the flow lock, this is held only for one commit — a read, the checks
//! and one `WriteBatch` — so it needs no renewal: its TTL is a crash backstop.
//! The long-lived exclusivity (a driver holding a run across a model call) is
//! the EXECUTION lease inside the record, fenced by `lease_epoch`; this lock
//! only makes each commit's check-and-write atomic cluster-wide.

use std::sync::Arc;
use std::time::Duration;

use raisin_agent_runtime::store::StoreError;
use raisin_locks::{FencingToken, LockManagerHandle};

use crate::jobs::keyed_mutex::{KeyedMutex, KeyedMutexGuard};

/// Lease TTL: a crash backstop only, commits take microseconds.
const COMMIT_LEASE_TTL: Duration = Duration::from_secs(10);
/// How long to try for the distributed lease before answering `Busy`.
const ACQUIRE_BUDGET: Duration = Duration::from_secs(3);
/// Backoff between attempts.
const RETRY_DELAY: Duration = Duration::from_millis(10);

/// `(tenant, repo, id)`.
pub(super) type LockKey = (String, String, String);

/// Hands out commit critical sections.
pub(super) struct RunLocks {
    kind: &'static str,
    local: Arc<KeyedMutex<LockKey>>,
    distributed: Option<LockManagerHandle>,
    owner: String,
}

/// Held for one commit; releases the distributed lease on drop.
pub(super) struct RunLockGuard {
    lease: Option<(LockManagerHandle, String, FencingToken)>,
    _local: KeyedMutexGuard<LockKey>,
}

impl Drop for RunLockGuard {
    fn drop(&mut self) {
        if let Some((manager, key, token)) = self.lease.take() {
            // Idempotent and token-checked: worst case the lease just expires.
            tokio::spawn(async move {
                let _ = manager.release(&key, token).await;
            });
        }
    }
}

impl RunLocks {
    /// Locks of one `kind` (`"run"` / `"subject"`), cluster-wide when
    /// `distributed` is set.
    pub(super) fn new(
        kind: &'static str,
        distributed: Option<LockManagerHandle>,
        owner: String,
    ) -> Self {
        Self {
            kind,
            local: Arc::new(KeyedMutex::new()),
            distributed,
            owner,
        }
    }

    /// Whether commits are serialized across nodes.
    pub(super) fn is_distributed(&self) -> bool {
        self.distributed.is_some()
    }

    fn lease_key(&self, key: &LockKey) -> String {
        raisin_locks::scoped_key(
            &key.0,
            &key.1,
            "-",
            &format!("agent_run_{}:{}", self.kind, key.2),
        )
    }

    /// Enter the critical section of `key`.
    pub(super) async fn lock(&self, key: LockKey) -> Result<RunLockGuard, StoreError> {
        let local = self.local.lock(key.clone()).await;
        let Some(manager) = &self.distributed else {
            return Ok(RunLockGuard {
                lease: None,
                _local: local,
            });
        };
        let lease_key = self.lease_key(&key);
        let deadline = tokio::time::Instant::now() + ACQUIRE_BUDGET;
        loop {
            match manager
                .try_acquire(&lease_key, &self.owner, COMMIT_LEASE_TTL)
                .await
            {
                Ok(Some(guard)) => {
                    return Ok(RunLockGuard {
                        lease: Some((manager.clone(), lease_key, guard.token)),
                        _local: local,
                    })
                }
                Ok(None) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(RETRY_DELAY).await
                }
                Ok(None) => return Err(StoreError::Busy),
                // The lock backend is down: the same call the flow lock makes —
                // proceed on the in-process guard and say so loudly, rather than
                // stalling every run in the cluster.
                Err(e) => {
                    tracing::warn!(
                        key = %lease_key,
                        error = %e,
                        "agent run commit lock backend unavailable - in-process serialization only"
                    );
                    return Ok(RunLockGuard {
                        lease: None,
                        _local: local,
                    });
                }
            }
        }
    }
}
