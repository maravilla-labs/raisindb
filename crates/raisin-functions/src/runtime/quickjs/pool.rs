// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Pooled QuickJS `AsyncRuntime`s with exclusive checkout.
//!
//! # Why a pool
//!
//! Creating an `rquickjs::AsyncRuntime` per function invocation churns the
//! allocator and bloats glibc arenas on the long-lived multi-tenant server
//! (~2.5 GB/day RSS growth in production). This pool keeps a small set of
//! warm runtimes and hands one out per execution instead.
//!
//! # Tenant-isolation invariants (LOAD-BEARING)
//!
//! A pooled runtime is reused across executions belonging to **different
//! customers**, so correctness depends on every execution resetting all
//! runtime-level state. These invariants are enforced jointly here and in
//! [`super::QuickJsRuntime::execute`]:
//!
//! 1. **Exclusive checkout.** A runtime is owned by exactly one execution for
//!    its whole duration ([`QuickJsPool::checkout`] pops it out of the idle
//!    set; it is only returned on [`Checkout`] drop). Two executions can never
//!    share a runtime concurrently — required because `set_loader` /
//!    `set_interrupt_handler` / `set_memory_limit` mutate *runtime*-global
//!    state, not context-local state.
//! 2. **Fresh context per execution.** `execute` builds a new `AsyncContext`
//!    and drops it before the [`Checkout`] returns the runtime, so no globals
//!    (`globalThis.*`) leak forward.
//! 3. **Loader replaced every execution.** `execute` calls `set_loader` on
//!    *every* run (empty file map when the function ships no files), so a
//!    runtime that previously served modules can never resolve a stale path
//!    for the next tenant.
//! 4. **Interrupt handler + memory limit re-set every execution**, never
//!    inherited from the previous occupant.
//! 5. **Recycle after N executions.** A pooled runtime is dropped and rebuilt
//!    once it has served [`QuickJsPool::recycle_threshold`] executions, to
//!    bound QuickJS-internal accumulation (atom table, shape caches) — both a
//!    memory bound and defense-in-depth against cross-tenant state bleed.
//! 6. **Poisoning.** An execution that ends in anything other than a clean
//!    success (Rust error, timeout, or wall-clock interrupt firing) DROPS its
//!    runtime instead of returning it — a half-interrupted interpreter may
//!    hold weird internal state, so we never reuse it.

use raisin_error::{Error, Result};
use rquickjs::AsyncRuntime;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use tokio::sync::{Semaphore, SemaphorePermit};

/// Default number of executions a pooled runtime serves before it is recycled.
pub(super) const DEFAULT_RECYCLE_THRESHOLD: usize = 500;

/// Pool capacity, aligned with the function-execution concurrency limit
/// (`RAISIN_MAX_CONCURRENT_FUNCTIONS`, default 15). The
/// `FUNCTION_EXECUTION_SEMAPHORE` already bounds how many executions run at
/// once, so a pool of equal size is never a bottleneck; checkouts still wait
/// on the pool's own permits if capacity were ever exceeded.
fn default_pool_capacity() -> usize {
    std::env::var("RAISIN_MAX_CONCURRENT_FUNCTIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15)
        .max(1)
}

/// Process-wide pool shared by every `QuickJsRuntime::new()` (hence by all ~9
/// `FunctionExecutor::new()` call sites across the transports).
pub(super) static GLOBAL_QUICKJS_POOL: LazyLock<std::sync::Arc<QuickJsPool>> =
    LazyLock::new(|| {
        std::sync::Arc::new(QuickJsPool::new(
            default_pool_capacity(),
            DEFAULT_RECYCLE_THRESHOLD,
        ))
    });

/// A warm runtime plus the number of executions it has served.
struct PooledRuntime {
    runtime: AsyncRuntime,
    /// Executions performed on this runtime (incremented at checkout).
    uses: usize,
}

/// A bounded pool of warm QuickJS `AsyncRuntime`s.
pub(super) struct QuickJsPool {
    /// Warm runtimes available for checkout.
    idle: Mutex<Vec<PooledRuntime>>,
    /// Bounds the total number of live runtimes; a checkout waits here when
    /// every runtime is currently checked out.
    permits: Semaphore,
    /// Total runtimes ever created — test hook for asserting recycle/poison.
    created: AtomicU64,
    /// Executions after which a runtime is dropped and rebuilt.
    recycle_threshold: usize,
}

impl QuickJsPool {
    /// Build a pool with `capacity` concurrent runtimes that recycles a runtime
    /// after `recycle_threshold` executions.
    pub(super) fn new(capacity: usize, recycle_threshold: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            idle: Mutex::new(Vec::with_capacity(capacity)),
            permits: Semaphore::new(capacity),
            created: AtomicU64::new(0),
            recycle_threshold: recycle_threshold.max(1),
        }
    }

    /// Total number of `AsyncRuntime`s this pool has ever created. Rises when a
    /// checkout has to build a fresh runtime (cold start, after a recycle, or
    /// after a poisoned runtime was dropped). Used by tests.
    pub(super) fn created_count(&self) -> u64 {
        self.created.load(Ordering::Relaxed)
    }

    /// Check out a runtime for the exclusive use of one execution.
    ///
    /// Waits for a capacity permit, then reuses a warm runtime or builds a new
    /// one. The returned [`Checkout`] must be marked healthy
    /// ([`Checkout::mark_healthy`]) for the runtime to return to the pool;
    /// otherwise it is dropped (poisoned).
    pub(super) async fn checkout(&self) -> Result<Checkout<'_>> {
        let permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| Error::Internal("QuickJS runtime pool closed".to_string()))?;

        let reused = {
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            idle.pop()
        };

        let mut pooled = match reused {
            Some(p) => p,
            None => {
                let runtime = AsyncRuntime::new().map_err(|e| {
                    Error::Internal(format!("Failed to create QuickJS runtime: {}", e))
                })?;
                self.created.fetch_add(1, Ordering::Relaxed);
                PooledRuntime { runtime, uses: 0 }
            }
        };
        pooled.uses += 1;

        Ok(Checkout {
            pool: self,
            _permit: permit,
            pooled: Some(pooled),
            healthy: false,
        })
    }

    fn checkin(&self, pooled: PooledRuntime) {
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        idle.push(pooled);
    }
}

/// Exclusive handle to a pooled runtime, held for one execution.
///
/// On drop the runtime is returned to the pool **only** if it was marked
/// healthy and has not reached the recycle threshold; otherwise it is dropped
/// so a fresh runtime is built next time (see the poisoning / recycle
/// invariants on [`QuickJsPool`]).
pub(super) struct Checkout<'a> {
    pool: &'a QuickJsPool,
    /// Held for the checkout's lifetime; releasing it frees a capacity slot.
    _permit: SemaphorePermit<'a>,
    pooled: Option<PooledRuntime>,
    healthy: bool,
}

impl Checkout<'_> {
    /// The checked-out runtime. Returns a cheap clone of the shared
    /// `AsyncRuntime` handle (rquickjs `AsyncRuntime` is `Clone` and clones
    /// point at the same interpreter); the pool retains the canonical handle.
    pub(super) fn runtime(&self) -> AsyncRuntime {
        self.pooled
            .as_ref()
            .expect("runtime present for the lifetime of a checkout")
            .runtime
            .clone()
    }

    /// Mark the execution as a clean success, allowing the runtime to be
    /// returned to the pool. Left unmarked (the default) the runtime is
    /// treated as poisoned and dropped on return.
    pub(super) fn mark_healthy(&mut self) {
        self.healthy = true;
    }
}

impl Drop for Checkout<'_> {
    fn drop(&mut self) {
        if let Some(pooled) = self.pooled.take() {
            let recycle = pooled.uses >= self.pool.recycle_threshold;
            if self.healthy && !recycle {
                self.pool.checkin(pooled);
            }
            // Otherwise `pooled` (and its `AsyncRuntime`) is dropped here,
            // freeing the interpreter; the next checkout builds a fresh one.
        }
        // `_permit` releases the capacity slot on drop.
    }
}
