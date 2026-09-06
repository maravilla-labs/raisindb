//! The in-memory counters behind the activity node.
//!
//! See the module docs in `super` for why this exists and what it must not do.

use super::snapshot::{self, Snapshot};
use super::{publisher::PublishSink, ScopeKey};
use raisin_storage::jobs::{JobContext, JobId, JobType};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

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
    pub(super) sink: OnceLock<PublishSink>,
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

        // Engine work is dropped HERE, before it reaches the counters — not
        // filtered at render time. Filtering later would leave `active` and the
        // path list disagreeing, which is how "Processing 20" ends up over a
        // list of two.
        if !is_tenant_visible(&context.workspace_id) {
            return ActivityGuard {
                tracker: Arc::clone(self),
                key,
                job_id: job_id.clone(),
                tracked: false,
            };
        }

        // A job this surface cannot NAME is not counted either. The record's
        // promise is that `active` and `active_paths` describe the same work;
        // a function run or a flow execution has no content subject, and
        // counting it produced "Processing 1 item(s) / and 1 more" over an
        // EMPTY list — a number pointing at nothing, which reads as a fault.
        // A subject is either there or the job is invisible, both halves at
        // once. (Only the cap can still make the list shorter than the count.)
        let Some(node_id) = subject_node_id(job_type) else {
            return ActivityGuard {
                tracker: Arc::clone(self),
                key,
                job_id: job_id.clone(),
                tracked: false,
            };
        };

        let entry = InFlight {
            workspace: context.workspace_id.clone(),
            node_id: Some(node_id.to_string()),
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
            tracked: true,
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

    /// Does THIS tenant have work parked against an upstream that is still
    /// down, in any of its repositories?
    ///
    /// The tenant-facing `degraded` bit, and it goes through the SAME
    /// derivation as the activity node — `any_parked_upstream_down` over the
    /// keys the handler itself parked on. Two derivations of one bit is how a
    /// banner and an endpoint end up disagreeing about whether a tenant is
    /// affected, with no error anywhere to say which is lying.
    ///
    /// Deliberately NOT "is any breaker open on this host": a breaker is shared
    /// by every tenant on the box, so the host-wide reading alarms tenants who
    /// have nothing in flight and tenants who never touch that upstream. Tying
    /// the bit to AFFECTED WORK means an unaffected tenant is told nothing,
    /// which is also what keeps the shared upstream's identity out of the
    /// answer.
    ///
    /// Aggregated across the tenant's repos because the caller asked about a
    /// TENANT: the endpoint is not repo-scoped, and reporting only the first
    /// repo would answer a question nobody asked.
    pub fn tenant_degraded(&self, tenant: &str) -> bool {
        let scopes = self.scopes.lock().unwrap_or_else(|e| e.into_inner());
        scopes
            .iter()
            .filter(|((scope_tenant, _repo), _)| scope_tenant == tenant)
            .any(|(_, state)| {
                snapshot::any_parked_upstream_down(
                    state.parked_upstreams.iter().map(String::as_str),
                )
            })
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
    /// False for engine work that was never counted. `finish` on an id that was
    /// never inserted is harmless today, but a guard that silently relies on
    /// that is one refactor away from decrementing something it never
    /// incremented — the unbalanced counter this type exists to prevent.
    tracked: bool,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if !self.tracked {
            return;
        }
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

/// Workspaces whose work is ENGINE business, not the tenant's.
///
/// The surface answers "is MY content being processed". A package install
/// re-indexes every function node it ships and a media job writes its own
/// bookkeeping — both are real work, neither is anything a person put there or
/// can act on. Surfaced, they drown the signal: one install showed "Processing
/// 20 items / 431 waiting", every line a `/lib/studio/...` path, while the two
/// assets the user actually uploaded were somewhere in the "and 12 more".
///
/// A DENYLIST rather than an allowlist on purpose. An allowlist would hide a
/// tenant's own content the day they add a workspace — silence about their work
/// is the failure this whole surface exists to prevent, whereas a new ENGINE
/// workspace leaking in is visible noise someone will report. Fail toward
/// showing too much, never toward showing nothing.
const ENGINE_WORKSPACES: &[&str] = &[
    // Package payload: functions, triggers, mappers. Reindexed wholesale on
    // every install.
    "functions",
    // Roles, users, grants — rewritten by the same installs.
    "raisin:access_control",
    // Mount config, definition state, engine records.
    "raisin:system",
    // This surface's own node. Publishing would be a feedback display, though
    // the node type is inert so no job is enqueued for it anyway.
    "job_activity",
    // Media job bookkeeping. The ASSET is the subject a person recognises; the
    // job node beside it is an implementation detail with a nanoid for a name.
    "media_jobs",
    // Per-user editor session scratch.
    "edit-sessions",
];

/// Is this work worth telling the tenant about?
fn is_tenant_visible(workspace: &str) -> bool {
    !ENGINE_WORKSPACES.contains(&workspace)
}

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
