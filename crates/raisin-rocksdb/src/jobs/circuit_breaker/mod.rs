//! A circuit breaker keyed by UPSTREAM, shared by every job in the process.
//!
//! # The incident this exists for
//!
//! 2026-09-02: an extraction-generation bump re-opened 257 assets spanning
//! every tenant on the host while the embedding upstream returned 503 for
//! nearly every call. 2,512 attempts, 3 successes, 476 upstream 503s, 96% CPU
//! on 16 cores. The concurrency cap held throughout — the damage was that every
//! job rediscovered the outage ALONE, and each one did the expensive work
//! (fetch, extract, chunk, tokenize) first and only then found out the upstream
//! was down. The cheap failure happened last.
//!
//! So the breaker's job is not to make the failure faster. It is to make the
//! failure happen BEFORE the CPU is spent, once, on behalf of everyone.
//!
//! # Keyed by upstream, never by tenant
//!
//! An upstream outage is not a tenant's fault, and every tenant must learn it
//! at once — that is the entire economy of a breaker. Keying by tenant would
//! make N tenants each pay the full discovery cost, which is what today already
//! does.
//!
//! The mirror image matters just as much: because one breaker parks EVERY
//! tenant's jobs, only a fault that genuinely belongs to the upstream may open
//! it. A 4xx is ours (bad model name, expired key — and keys are per tenant),
//! so it must not. That rule lives in [`raisin_error::Error::is_upstream_fault`]
//! rather than here, so the read path can one day ask the same question.
//!
//! # Per process, like everything else in the job system
//!
//! Job dedup and the handler pools are per-process, so on an N-node cluster
//! each node discovers an outage once. That is deliberate and out of scope
//! (see `docs/plans/fair-job-scheduling.md`, "Multi-node caveat"): the resource
//! being protected is this box's CPU.
//!
//! # Not to be confused with `TriggerBreaker`
//!
//! `handlers::trigger_evaluation::breaker` is a RATE limiter — it stops a
//! trigger firing too often. This is a FAULT breaker: it stops work being
//! attempted against something that is down. They share a word and nothing
//! else.

mod state;
#[cfg(test)]
mod tests;

pub use state::{BreakerPolicy, BreakerState, BreakerStatus};

use raisin_error::{Error, Result};
use state::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The process-wide breaker registry.
///
/// # Locking
///
/// Two levels, both with critical sections measured in nanoseconds and NEVER
/// held across an `await` — this codebase has been burned by that before, and a
/// lock held over a provider call would serialise the very requests the breaker
/// exists to avoid making.
///
/// * an `RwLock` over the key→cell map, taken for READ on every call and for
///   write only the first time a key is seen (upstreams are a handful, so the
///   write path effectively runs once per process);
/// * a `Mutex` per cell, guarding a struct of six `Copy` fields.
///
/// Every method here is synchronous and allocation-light on the hot path, so
/// there is nothing to await inside either.
pub struct CircuitBreakerRegistry {
    policy: BreakerPolicy,
    cells: RwLock<HashMap<String, Arc<Mutex<Cell>>>>,
}

static GLOBAL: OnceLock<Arc<CircuitBreakerRegistry>> = OnceLock::new();

impl CircuitBreakerRegistry {
    /// The one registry every job handler consults.
    ///
    /// A process-wide singleton rather than a field threaded through every
    /// handler constructor, because the sharing IS the feature: a second
    /// registry would mean two populations of jobs each discovering the same
    /// outage. Tests build their own with [`CircuitBreakerRegistry::with_policy`]
    /// so they neither see nor pollute this one.
    pub fn global() -> &'static Arc<CircuitBreakerRegistry> {
        GLOBAL
            .get_or_init(|| Arc::new(CircuitBreakerRegistry::with_policy(BreakerPolicy::default())))
    }

    pub fn with_policy(policy: BreakerPolicy) -> Self {
        Self {
            policy,
            cells: RwLock::new(HashMap::new()),
        }
    }

    /// May this job attempt work against `key`?
    ///
    /// Called ONCE per job, before anything expensive. `Err` is
    /// [`Error::UpstreamUnavailable`], which the job worker parks on rather
    /// than retrying — nothing was attempted, so nothing failed.
    ///
    /// This is also the transition point out of `Open`: the first caller after
    /// the cooldown becomes the single half-open PROBE and is let through;
    /// everyone else keeps parking until that probe reports.
    pub fn admit(&self, key: &str) -> Result<()> {
        let cell = self.cell(key);
        let mut cell = cell.lock().unwrap_or_else(|e| e.into_inner());
        match cell.admit(&self.policy) {
            None => Ok(()),
            Some(retry_after) => Err(self.unavailable(key, retry_after)),
        }
    }

    /// Has the breaker gone OPEN since this job was admitted?
    ///
    /// Read-only: it never claims a probe. A job that is already inside its
    /// attempt calls this between units of work (each embedding spec chunks its
    /// own text) so a breaker tripped by the first unit stops the second from
    /// chunking at all. A half-open state answers `Ok` because the caller is
    /// the probe holder.
    pub fn check(&self, key: &str) -> Result<()> {
        let cell = self.cell(key);
        let cell = cell.lock().unwrap_or_else(|e| e.into_inner());
        match cell.remaining_cooldown(&self.policy) {
            None => Ok(()),
            Some(retry_after) => Err(self.unavailable(key, retry_after)),
        }
    }

    /// The upstream answered correctly. Closes the breaker unconditionally.
    pub fn record_success(&self, key: &str) {
        let cell = self.cell(key);
        let mut cell = cell.lock().unwrap_or_else(|e| e.into_inner());
        cell.record_success();
    }

    /// An attempt against `key` failed.
    ///
    /// Classification is delegated to [`Error::is_upstream_fault`], so what
    /// opens a breaker is decided in ONE place and the read path can ask the
    /// same question later. A non-fault (4xx, a serialization error, our own
    /// bug) leaves a closed breaker alone and CLOSES a half-open one: the
    /// upstream demonstrably answered, which is exactly what the probe was
    /// asking.
    pub fn record_failure(&self, key: &str, error: &Error) {
        let cell = self.cell(key);
        let mut cell = cell.lock().unwrap_or_else(|e| e.into_inner());
        cell.record_failure(&self.policy, error.is_upstream_fault(), error.to_string());
    }

    /// One breaker's state, for the admin surface. `None` if never used.
    pub fn status(&self, key: &str) -> Option<BreakerStatus> {
        let cells = self.cells.read().unwrap_or_else(|e| e.into_inner());
        let cell = cells.get(key)?;
        let cell = cell.lock().unwrap_or_else(|e| e.into_inner());
        Some(cell.status(key.to_string(), &self.policy))
    }

    /// Every breaker this process has touched. The read side of the admin
    /// endpoint in `docs/plans/fair-job-scheduling.md` §D — a breaker that is
    /// open and invisible is a silent stall, which is worse than the outage.
    pub fn statuses(&self) -> Vec<BreakerStatus> {
        let cells = self.cells.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<BreakerStatus> = cells
            .iter()
            .map(|(key, cell)| {
                let cell = cell.lock().unwrap_or_else(|e| e.into_inner());
                cell.status(key.clone(), &self.policy)
            })
            .collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    fn unavailable(&self, key: &str, retry_after: std::time::Duration) -> Error {
        Error::UpstreamUnavailable {
            upstream: key.to_string(),
            // Round UP, so a sub-second remainder never parks a job into the
            // past and produces a hot re-park loop.
            retry_after_secs: retry_after.as_secs().max(1),
        }
    }

    /// The cell for `key`, created on first sight.
    ///
    /// Read-locked in the common case. The double-check on the write path
    /// matters: two jobs racing the first use of an upstream must end up
    /// sharing ONE cell, or the first N failures are split across two counters
    /// and the breaker opens late — exactly when the burst is largest.
    fn cell(&self, key: &str) -> Arc<Mutex<Cell>> {
        if let Some(cell) = self
            .cells
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
        {
            return Arc::clone(cell);
        }
        let mut cells = self.cells.write().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            cells
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(Cell::default()))),
        )
    }
}
