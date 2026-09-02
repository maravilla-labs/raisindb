//! What one repository's activity looks like right now, and when that is worth
//! writing down.

use crate::jobs::circuit_breaker::{BreakerState, CircuitBreakerRegistry};

/// How many in-flight subjects the activity node names.
///
/// A CAP, not a page size. The property is there so the SPA can say "working
/// on these", not so anyone can reconstruct the queue — an uncapped list would
/// turn a 2,000-job burst into a 2,000-entry property on a replicated node, and
/// the node would grow with the incident it is meant to describe.
pub const ACTIVE_PATHS_CAP: usize = 8;

/// One repository's activity, as this process sees it.
///
/// Everything here is derived WITHOUT a storage read, because it is computed on
/// every job start and finish. Turning a subject into a path costs a read and
/// happens only when a write is actually going to occur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Whether THIS repository has background work parked against an upstream
    /// that is currently down.
    ///
    /// # Not "an upstream is down somewhere on this host"
    ///
    /// Breakers are keyed by UPSTREAM and shared across every tenant on the
    /// box; tenants are not. So a bit derived from "is any breaker open" would
    /// alarm every tenant on the machine, including tenants with nothing in
    /// flight and tenants that do not use embeddings at all. The banner's claim
    /// to the reader is "YOUR processing is paused and will catch up", and for
    /// those tenants that claim is simply false — a false alarm that trains
    /// people to ignore the indicator, which costs us the next real incident.
    ///
    /// So the bit is tied to AFFECTED WORK: a scope is degraded only while it
    /// has a job parked against a key whose breaker is not closed. That removes
    /// the fan-out problem rather than managing it — no affected work means no
    /// write, means no node update, means no banner, with nothing to suppress —
    /// and it composes with the idle rule, since an idle tenant writes nothing
    /// and therefore cannot be alarmed by someone else's outage. A tenant who
    /// uploads DURING an outage learns as soon as their job parks, which is
    /// exactly the moment the claim becomes true.
    ///
    /// # What it deliberately does NOT carry
    ///
    /// One bit. No upstream identity, no consecutive-failure count, no cooldown
    /// or "next probe in N seconds" — a shared upstream's probe timer is a
    /// fingerprint of OTHER tenants' traffic against it. The operator surface
    /// carries those; the tenant gets the bit.
    ///
    /// # Per process
    ///
    /// Breakers are per-process, so the honest reading is "degraded on the node
    /// that answered", not "degraded everywhere". Nothing here promises more:
    /// the name says degraded, not degraded-cluster-wide, and the node's
    /// `origin` says whose view this is.
    pub degraded: bool,
    /// Job handlers executing for this repository in this process.
    pub active: usize,
    /// Up to [`ACTIVE_PATHS_CAP`] `(workspace, node_id)` pairs currently being
    /// worked on, sorted so that two snapshots of the same work compare equal
    /// regardless of `HashMap` iteration order. Without the sort, every job
    /// start would look like a change and the coalescing would never settle.
    pub subjects: Vec<(String, String)>,
}

impl Snapshot {
    /// Build a snapshot from one scope's in-flight jobs and the upstream keys
    /// it currently has work parked against.
    pub fn build<'a>(
        in_flight: impl Iterator<Item = (&'a str, Option<&'a str>)>,
        parked_upstreams: impl Iterator<Item = &'a str>,
    ) -> Self {
        Self::build_with_degraded(in_flight, any_parked_upstream_down(parked_upstreams))
    }

    /// [`Snapshot::build`] with the degraded flag supplied rather than read
    /// from the process-wide breaker registry — the seam the unit tests use, so
    /// they neither see nor pollute the real one.
    pub fn build_with_degraded<'a>(
        in_flight: impl Iterator<Item = (&'a str, Option<&'a str>)>,
        degraded: bool,
    ) -> Self {
        let mut active = 0usize;
        let mut subjects: Vec<(String, String)> = Vec::new();
        for (workspace, node_id) in in_flight {
            active += 1;
            if let Some(node_id) = node_id {
                subjects.push((workspace.to_string(), node_id.to_string()));
            }
        }
        subjects.sort();
        subjects.dedup();
        subjects.truncate(ACTIVE_PATHS_CAP);
        Snapshot {
            degraded,
            active,
            subjects,
        }
    }

    /// Nothing is happening and nothing is wrong.
    ///
    /// The condition under which a scope is dropped from the tracker's map
    /// entirely, so that map holds live repositories rather than every
    /// repository this process has ever touched.
    pub fn is_idle(&self) -> bool {
        !self.degraded && self.active == 0 && self.subjects.is_empty()
    }

    /// Is this worth a node revision?
    ///
    /// The whole write-on-transition rule reduces to this call. Note what it
    /// does NOT compare: wall-clock time. A snapshot identical to the published
    /// one is not written however long ago that publish was, which is what
    /// makes an idle tenant cost zero writes forever.
    pub fn differs_from(&self, published: Option<&Snapshot>) -> bool {
        match published {
            None => !self.is_idle(),
            Some(prev) => prev != self,
        }
    }
}

/// Whether any of the upstreams this scope has work parked against is still
/// down.
///
/// The keys are the ones the embedding handler ALREADY BUILT for the `admit()`
/// call that parked the job — `handlers::embedding::upstream::upstream_key`,
/// carried here verbatim. They are never re-derived, and never matched by
/// prefix or pattern: a key rebuilt independently would drift from the one
/// actually dialled, and this bit would then read "healthy" straight through an
/// outage with no error anywhere.
///
/// `status(key) == None` — no breaker for that key exists — is NOT degraded. An
/// upstream that has never been called is not an upstream that is down, and a
/// fresh tenant must not see a banner because no embedding has ever run. Since
/// a key only gets here by way of a park, and a park requires an open breaker,
/// `None` in practice means the breaker was pruned; either way the answer is
/// the same.
///
/// Half-open counts as degraded: the probe has not reported yet, so the parked
/// work is still parked for everyone but the one job holding it.
pub(super) fn any_parked_upstream_down<'a>(keys: impl Iterator<Item = &'a str>) -> bool {
    let registry = CircuitBreakerRegistry::global();
    keys.filter_map(|key| registry.status(key))
        .any(|s| s.state != BreakerState::Closed)
}

/// Whether one upstream key's breaker has recovered, so the scope may forget
/// it. A key with no breaker at all has nothing to wait for.
pub(super) fn upstream_recovered(key: &str) -> bool {
    CircuitBreakerRegistry::global()
        .status(key)
        .is_none_or(|s| s.state == BreakerState::Closed)
}
