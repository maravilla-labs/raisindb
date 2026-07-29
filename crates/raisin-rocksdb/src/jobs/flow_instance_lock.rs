// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Single-writer serialization for flow instance execution.
//!
//! # Why this exists
//!
//! A flow instance is a single aggregate that several producers legitimately
//! try to advance at the same instant: sibling parallel branches reporting
//! terminal, a function result landing while a human task completes, a
//! timeout check firing next to a resume. The runtime's answer to that is
//! optimistic concurrency — `save_instance_with_version` plus an inline
//! `VersionConflict` retry in the job handler — which is the right design
//! but relies on the version check being ATOMIC.
//!
//! It was not: the check loaded the instance, compared the version, and
//! wrote in three separate steps, so two writers both read version N, both
//! passed the check, and both wrote N+1. The second write carried a snapshot
//! taken before the first, so the first writer's accumulated state was
//! silently lost — and a parallel join waiting on that state waited forever.
//!
//! Rather than make every flow write conditional (the instance is stored as
//! an ordinary node, through the generic node saver), this serializes
//! execution per instance so there is only ever one writer. That is the same
//! shape mature engines use — one outstanding task per workflow execution,
//! or single-threaded command processing per instance partition — and it
//! makes the optimistic path above correct instead of decorative.
//!
//! # Two layers
//!
//! - An **in-process** keyed mutex, always present. Queues concurrent
//!   executions of the same instance inside one node, which is the whole
//!   problem on a single-node deployment.
//! - An optional **distributed lease** from `raisin-locks`, when the locks
//!   subsystem is configured. Required for correctness across a cluster:
//!   with the `inprocess` lock backend, or with locks disabled, two NODES
//!   can still execute the same instance concurrently.
//!
//! The lease is renewed while execution runs and released on drop, so a
//! crashed worker's lock expires instead of wedging the instance.

use std::sync::Arc;
use std::time::Duration;

use raisin_locks::{FencingToken, LockManagerHandle};

use crate::jobs::keyed_mutex::{KeyedMutex, KeyedMutexGuard};

/// Lease duration for the distributed lock.
///
/// Comfortably above the default per-step timeout (5 minutes) so a slow but
/// healthy step never loses its lease, and renewed while running anyway.
const LEASE_TTL: Duration = Duration::from_secs(600);

/// How often the lease is extended while execution is in flight.
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(120);

/// How long to keep trying for the distributed lease before giving up and
/// letting the job system redeliver. Contention here is normally a few
/// milliseconds (sibling branches finishing together).
const LEASE_ACQUIRE_BUDGET: Duration = Duration::from_secs(2);

/// Backoff between distributed-lease attempts.
const LEASE_RETRY_DELAY: Duration = Duration::from_millis(25);

/// Another worker holds the instance; the caller should let the job system
/// redeliver rather than execute concurrently.
#[derive(Debug)]
pub struct FlowInstanceBusy {
    /// The instance that is already being executed
    pub instance_id: String,
}

impl std::fmt::Display for FlowInstanceBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "flow instance {} is being executed by another worker",
            self.instance_id
        )
    }
}

/// Hands out per-instance execution locks.
pub struct FlowInstanceLockManager {
    /// Per-instance mutexes, keyed by the scoped lock key
    local: Arc<KeyedMutex<String>>,
    /// Optional cluster-wide lease provider
    distributed: Option<LockManagerHandle>,
}

impl FlowInstanceLockManager {
    /// Create a manager, optionally backed by the locks subsystem.
    pub fn new(distributed: Option<LockManagerHandle>) -> Self {
        Self {
            local: Arc::new(KeyedMutex::new()),
            distributed,
        }
    }

    /// The lock key for an instance, using the shared scoping convention.
    fn lock_key(tenant_id: &str, repo_id: &str, branch: &str, instance_id: &str) -> String {
        raisin_locks::scoped_key(
            tenant_id,
            repo_id,
            branch,
            &format!("flow_instance:{}", instance_id),
        )
    }

    /// Take the execution lock for one instance.
    ///
    /// Waits for the in-process mutex (concurrent deliveries queue rather
    /// than fail), then tries for the distributed lease within a short
    /// budget. Returns [`FlowInstanceBusy`] only when another NODE holds the
    /// instance past that budget.
    pub async fn acquire(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        instance_id: &str,
        owner: &str,
    ) -> std::result::Result<FlowInstanceLease, FlowInstanceBusy> {
        let key = Self::lock_key(tenant_id, repo_id, branch, instance_id);

        // In-process: queue behind any local execution of this instance.
        let local_guard = self.local.lock(key.clone()).await;

        // Distributed: only a cluster peer can be holding it now.
        let mut renewer = None;
        if let Some(manager) = &self.distributed {
            let deadline = tokio::time::Instant::now() + LEASE_ACQUIRE_BUDGET;
            let mut token = None;
            loop {
                match manager.try_acquire(&key, owner, LEASE_TTL).await {
                    Ok(Some(guard)) => {
                        token = Some(guard.token);
                        break;
                    }
                    Ok(None) => {
                        if tokio::time::Instant::now() >= deadline {
                            return Err(FlowInstanceBusy {
                                instance_id: instance_id.to_string(),
                            });
                        }
                        tokio::time::sleep(LEASE_RETRY_DELAY).await;
                    }
                    Err(e) => {
                        // The lock backend is unavailable. Proceed on the
                        // in-process guard alone rather than stalling every
                        // flow in the system: single-node stays correct, and
                        // a degraded lock backend is loudly logged.
                        tracing::warn!(
                            instance_id = %instance_id,
                            error = %e,
                            "Flow instance lock backend unavailable - proceeding with \
                             in-process serialization only"
                        );
                        break;
                    }
                }
            }

            if let Some(token) = token {
                renewer = Some(spawn_renewer(manager.clone(), key.clone(), token));
            }
        }

        Ok(FlowInstanceLease {
            renewer,
            _local_guard: local_guard,
        })
    }
}

/// Held for the duration of one instance execution; releases on drop.
pub struct FlowInstanceLease {
    /// Renew loop + release for the distributed lease.
    ///
    /// Declared FIRST so it drops first: releasing the lease before the local
    /// mutex means the next local waiter finds the lease free instead of
    /// polling the backend for one the previous holder has finished with.
    renewer: Option<LeaseRenewer>,
    /// Dropping this frees the in-process mutex
    _local_guard: KeyedMutexGuard<String>,
}

/// Keeps a distributed lease alive, and releases it when dropped.
struct LeaseRenewer {
    manager: LockManagerHandle,
    key: String,
    token: FencingToken,
    handle: tokio::task::JoinHandle<()>,
}

fn spawn_renewer(manager: LockManagerHandle, key: String, token: FencingToken) -> LeaseRenewer {
    let renew_manager = manager.clone();
    let renew_key = key.clone();
    let handle = tokio::spawn(async move {
        loop {
            tokio::time::sleep(LEASE_RENEW_INTERVAL).await;
            match renew_manager.renew(&renew_key, token, LEASE_TTL).await {
                Ok(true) => {}
                // Lease lost (expired or taken over) - stop renewing. The
                // execution will fail its own version check rather than
                // silently writing over a new holder.
                Ok(false) => {
                    tracing::warn!(
                        lock_key = %renew_key,
                        "Flow instance lease lost while executing"
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        lock_key = %renew_key,
                        error = %e,
                        "Failed to renew flow instance lease"
                    );
                    break;
                }
            }
        }
    });

    LeaseRenewer {
        manager,
        key,
        token,
        handle,
    }
}

impl Drop for LeaseRenewer {
    fn drop(&mut self) {
        self.handle.abort();
        let manager = self.manager.clone();
        let key = std::mem::take(&mut self.key);
        let token = self.token;
        // Release is idempotent and token-checked, so a best-effort
        // fire-and-forget is safe: worst case the lease simply expires.
        tokio::spawn(async move {
            if let Err(e) = manager.release(&key, token).await {
                tracing::debug!(lock_key = %key, error = %e, "Flow instance lease release failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_same_instance_serializes_locally() {
        let manager = Arc::new(FlowInstanceLockManager::new(None));

        let first = manager
            .acquire("t", "r", "main", "inst-1", "job-1")
            .await
            .expect("first acquire succeeds");

        // A second acquire for the SAME instance must not proceed while the
        // first is held - this is the lost-update guard.
        let contender = {
            let manager = manager.clone();
            tokio::spawn(async move { manager.acquire("t", "r", "main", "inst-1", "job-2").await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!contender.is_finished(), "second writer must wait");

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(2), contender)
            .await
            .expect("second writer proceeds once the first releases")
            .expect("task joins");
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn test_different_instances_do_not_block_each_other() {
        let manager = FlowInstanceLockManager::new(None);

        let _a = manager
            .acquire("t", "r", "main", "inst-a", "job-1")
            .await
            .expect("instance a");
        let b = tokio::time::timeout(
            Duration::from_millis(200),
            manager.acquire("t", "r", "main", "inst-b", "job-2"),
        )
        .await
        .expect("a different instance must not block");
        assert!(b.is_ok());
    }

    #[tokio::test]
    async fn test_lock_key_is_scoped_per_tenant_repo_branch() {
        let a = FlowInstanceLockManager::lock_key("t1", "r", "main", "i");
        let b = FlowInstanceLockManager::lock_key("t2", "r", "main", "i");
        let c = FlowInstanceLockManager::lock_key("t1", "r", "dev", "i");
        assert_ne!(a, b, "tenants must not share an instance lock");
        assert_ne!(a, c, "branches must not share an instance lock");
        assert!(a.contains("flow_instance"));
    }
}
