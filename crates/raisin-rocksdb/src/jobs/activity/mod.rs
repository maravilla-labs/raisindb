//! Per-repository background-job activity, published as a NODE so a tenant can
//! see it.
//!
//! # Why a node and not an endpoint
//!
//! Studio runs as an ordinary tenant user. `/management/*` is operator-only,
//! and widening it would hand every tenant the upstream hostnames and the
//! host-wide, cross-tenant pool counters that live behind it. Writing a
//! per-tenant node instead gets isolation from the `{tenant}\0{repo}\0…` key
//! prefix for free, and gets realtime PUSH through the WebSocket node
//! subscription the SPA already holds — no polling, no new authorization
//! surface.
//!
//! # The trap this module is built around
//!
//! A node write emits `node:updated`, which is exactly what enqueues fulltext
//! indexing, embedding, asset processing and trigger evaluation. A naive "write
//! a node per job event" design is therefore a feedback loop AND an amplifier:
//! the 2026-09-02 incident produced 2,512 job events in five minutes, and one
//! node write each would have produced 2,512 node revisions, each enqueueing
//! further jobs — a worse outage than the one this work exists to prevent.
//!
//! Three defences, all of them load-bearing:
//!
//! 1. **Bounded cardinality.** ONE node per tenant per repo, overwritten in
//!    place at `job_activity:/activity`. Its properties SUMMARISE; nothing here
//!    enumerates history, and `active_paths` is capped.
//! 2. **An inert node type.** `raisin:JobActivity` declares
//!    `index_types: [Property]` — no `Fulltext`, no `Vector` — so the two arms
//!    of `UnifiedJobEventHandler::handle_node_change` that would enqueue work
//!    are gated off at the source. The write also carries the `bookkeeping`
//!    marker, which is what skips trigger evaluation. See
//!    `event_handler::tests::activity_node_write_enqueues_no_indexing_jobs`,
//!    which is the regression test for the whole loop.
//! 3. **Write on TRANSITION, never on a timer.** [`Snapshot::differs_from`]
//!    decides whether anything worth reporting changed; if nothing did, nothing
//!    is written. An IDLE TENANT PRODUCES ZERO WRITES — otherwise every tenant
//!    mints revisions forever and MVCC history grows without bound. Writes that
//!    do happen are coalesced behind [`MIN_PUBLISH_INTERVAL`].
//!
//! # Per-process counters, and why the node is last-writer-wins
//!
//! These counters — like the job pools, the dedup map and the circuit breaker —
//! are PER-PROCESS. On an N-node cluster each node has its own view.
//!
//! The node is nevertheless ONE node per repo, last-writer-wins, rather than
//! one per cluster member. Cardinality is the harder constraint here: a
//! per-member path multiplies the write rate, the replication traffic and the
//! MVCC history by the cluster size, for a surface that is ambient and
//! approximate by construction — "is my content being processed, and is
//! anything stuck". The SPA would also have to merge N documents to answer a
//! question that has one answer. `origin` records whose view won, so the LWW is
//! attributable rather than anonymous. Cluster coordination is deliberately not
//! built; see `docs/plans/fair-job-scheduling.md`, "Multi-node caveat".

mod publisher;
mod snapshot;

#[cfg(test)]
mod tests;

pub use publisher::{install_publisher, ACTIVITY_ACTOR, ACTIVITY_BRANCH, ACTIVITY_NODE_PATH};
pub use snapshot::{Snapshot, ACTIVE_PATHS_CAP};

use publisher::PublishSink;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Floor between two writes of the same repository's activity node.
///
/// Coalescing, not sampling: a transition inside the window is not dropped, it
/// is held and published when the window closes. Two seconds is short enough
/// that a person watching a progress strip sees it move, and long enough that a
/// burst of a thousand job starts still costs at most one node revision per
/// two seconds per repo.
pub const MIN_PUBLISH_INTERVAL: Duration = Duration::from_secs(2);

/// The workspace the activity node lives in. Declared in
/// `crates/raisin-core/global_workspaces/job_activity.yaml`.
pub const ACTIVITY_WORKSPACE: &str = "job_activity";

/// The node type of the activity node. Its `index_types: [Property]` is what
/// makes this whole mechanism inert — see the module docs.
pub const ACTIVITY_NODE_TYPE: &str = "raisin:JobActivity";

/// A tenant+repo pair, the granularity of one activity node.
pub type ScopeKey = (String, String);

/// One in-flight job, remembered only for as long as it runs.
#[derive(Debug, Clone)]
struct InFlight {
    workspace: String,
    /// The node this job is about, when it is about one at all. A maintenance
    /// job (rebuild, compaction, backup) has no subject and contributes to the
    /// counts without contributing a path.
    node_id: Option<String>,
}

/// What this process currently knows about one repository.
#[derive(Debug, Default)]
struct ScopeState {
    in_flight: HashMap<JobId, InFlight>,
    /// Upstream breaker keys this repository currently has work parked
    /// against, as produced by the handler that parked it. Never re-derived
    /// here — see `snapshot::any_parked_upstream_down`.
    parked_upstreams: HashSet<String>,
    /// The snapshot most recently WRITTEN. `None` until the first publish, so
    /// the first transition out of idle always writes.
    published: Option<Snapshot>,
}

/// Process-wide job activity, keyed by tenant+repo.
///
/// # Locking
///
/// One `Mutex` over the whole map, held for the few instructions it takes to
/// insert or remove an entry and compare two small structs — never across an
/// `await`, and never across the node write, which is spawned. Job starts and
/// finishes are frequent but the critical section is nanoseconds; a per-scope
/// lock would buy nothing and cost the ability to compare scopes atomically.
pub struct JobActivityTracker {
    scopes: Mutex<HashMap<ScopeKey, ScopeState>>,
    /// Where a publish goes. `None` until [`install_publisher`] runs, which is
    /// when the worker pool starts. Absent it the tracker still COUNTS — tests
    /// and any embedding of this crate without a running pool observe the
    /// counters and write nothing.
    sink: OnceLock<PublishSink>,
}

static GLOBAL: OnceLock<Arc<JobActivityTracker>> = OnceLock::new();

impl JobActivityTracker {
    /// The one tracker every job handler reports to.
    ///
    /// A process-wide singleton for the same reason the circuit breaker is one:
    /// the sharing IS the feature. Two trackers would each publish half a view
    /// over the same node and fight for last-writer-wins.
    pub fn global() -> &'static Arc<JobActivityTracker> {
        GLOBAL.get_or_init(|| Arc::new(JobActivityTracker::new()))
    }

    pub fn new() -> Self {
        Self {
            scopes: Mutex::new(HashMap::new()),
            sink: OnceLock::new(),
        }
    }

    /// Record that a handler has begun executing, and return a guard that
    /// records its completion.
    ///
    /// RAII rather than a matching `finished()` call: a handler can return
    /// early, return `Err`, park on an open breaker, or be aborted by the
    /// timeout watchdog, and an unbalanced counter here would pin the surface
    /// at a permanent false "busy" that no later event ever clears.
    pub fn track(
        self: &Arc<Self>,
        job_id: &JobId,
        job_type: &JobType,
        context: &JobContext,
    ) -> ActivityGuard {
        let key = (context.tenant_id.clone(), context.repo_id.clone());
        let entry = InFlight {
            workspace: context.workspace_id.clone(),
            node_id: subject_node_id(job_type).map(|s| s.to_string()),
        };
        {
            let mut scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
            scopes
                .entry(key.clone())
                .or_default()
                .in_flight
                .insert(job_id.clone(), entry);
        }
        self.maybe_publish(&key);
        ActivityGuard {
            tracker: Arc::clone(self),
            key,
            job_id: job_id.clone(),
        }
    }

    /// A job for this scope was PARKED because `upstream` is down.
    ///
    /// This — not a scan of the breaker registry — is what makes a tenant
    /// degraded, and it is the whole of the fan-out rule. The key passed in is
    /// the one the caller already built for its own `admit()` call, so the bit
    /// can never watch a key nothing dials.
    ///
    /// A tenant with no parked work is never told about an outage on a shared
    /// upstream, because the banner's claim ("your processing is paused and
    /// will catch up") would not be true of them.
    pub fn record_parked(self: &Arc<Self>, context: &JobContext, upstream: &str) {
        let key = (context.tenant_id.clone(), context.repo_id.clone());
        {
            let mut scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
            scopes
                .entry(key.clone())
                .or_default()
                .parked_upstreams
                .insert(upstream.to_string());
        }
        self.maybe_publish(&key);
    }

    /// `upstream` answered correctly for this scope, so its work is moving
    /// again. Clearing here rather than only on the periodic re-check means the
    /// banner goes away on the first successful call, not on the next tick.
    pub fn record_upstream_ok(self: &Arc<Self>, context: &JobContext, upstream: &str) {
        let key = (context.tenant_id.clone(), context.repo_id.clone());
        let changed = {
            let mut scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
            match scopes.get_mut(&key) {
                Some(state) => state.parked_upstreams.remove(upstream),
                None => false,
            }
        };
        if changed {
            self.maybe_publish(&key);
        }
    }

    /// Take the current snapshot for one scope, or `None` if the scope is
    /// unknown to this process.
    pub(super) fn snapshot(&self, key: &ScopeKey) -> Option<Snapshot> {
        let scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
        let state = scopes.get(key)?;
        Some(Snapshot::build(
            state
                .in_flight
                .values()
                .map(|f| (f.workspace.as_str(), f.node_id.as_deref())),
            state.parked_upstreams.iter().map(String::as_str),
        ))
    }

    /// A transition may have happened: publish it, or schedule a coalesced
    /// flush, or do nothing at all.
    ///
    /// Doing nothing is the common case and the important one. An idle tenant
    /// reaches here only when something actually started or finished, and even
    /// then only writes if the resulting snapshot DIFFERS from what is stored.
    fn maybe_publish(&self, key: &ScopeKey) {
        let Some(sink) = self.sink.get() else {
            return;
        };
        sink.notify(key.clone());
    }

    /// Record what was just written, so the next transition can be compared
    /// against it. Returns the previously published snapshot.
    pub(super) fn set_published(&self, key: &ScopeKey, snapshot: Option<Snapshot>) {
        let mut scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = scopes.get_mut(key) {
            state.published = snapshot;
            // Forget the upstreams that have recovered. Done right after a
            // write, so the set holds only what the NEXT snapshot would still
            // call degraded — otherwise a key from a long-resolved outage would
            // keep a scope alive in this map forever.
            state
                .parked_upstreams
                .retain(|key| !snapshot::upstream_recovered(key));
            // An idle scope with nothing outstanding is dropped entirely: the
            // map must not accumulate one entry per repo this process has ever
            // touched.
            if state.in_flight.is_empty()
                && state.parked_upstreams.is_empty()
                && state.published.as_ref().is_none_or(|s| s.is_idle())
            {
                scopes.remove(key);
            }
        }
    }

    pub(super) fn published(&self, key: &ScopeKey) -> Option<Snapshot> {
        let scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
        scopes.get(key)?.published.clone()
    }

    fn finish(&self, key: &ScopeKey, job_id: &JobId) {
        {
            let mut scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = scopes.get_mut(key) {
                state.in_flight.remove(job_id);
            }
        }
        self.maybe_publish(key);
    }
}

impl Default for JobActivityTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Decrements the in-flight count for one job when dropped.
///
/// Held across the handler's whole execution, including a panic unwind.
pub struct ActivityGuard {
    tracker: Arc<JobActivityTracker>,
    key: ScopeKey,
    job_id: JobId,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.tracker.finish(&self.key, &self.job_id);
    }
}

/// The node a job is ABOUT, where it is about one.
///
/// Only the node-scoped job types are listed. A maintenance job (rebuild,
/// verify, compaction, backup) is repository-wide and contributes to the counts
/// without contributing a path — reporting a path for it would be inventing
/// one. Job types added later default to `None`, which degrades to "counted but
/// unnamed" rather than to a wrong path.
fn subject_node_id(job_type: &JobType) -> Option<&str> {
    match job_type {
        JobType::FulltextIndex { node_id, .. }
        | JobType::EmbeddingGenerate { node_id }
        | JobType::EmbeddingDelete { node_id }
        | JobType::NodeDeleteCleanup { node_id, .. }
        | JobType::RetargetReferences { node_id, .. }
        | JobType::TriggerEvaluation { node_id, .. }
        | JobType::AssetProcessing { node_id, .. } => Some(node_id.as_str()),
        _ => None,
    }
}
