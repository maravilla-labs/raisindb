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
mod tracker;

#[cfg(test)]
mod tests;

pub use publisher::{install_publisher, ACTIVITY_ACTOR, ACTIVITY_BRANCH, ACTIVITY_NODE_PATH};
pub use snapshot::{Snapshot, ACTIVE_PATHS_CAP};
pub use tracker::{ActivityGuard, JobActivityTracker};

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
