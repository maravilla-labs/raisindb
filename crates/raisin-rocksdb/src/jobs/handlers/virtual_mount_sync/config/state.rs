//! The engine-managed `state` blob of a `raisin:VirtualMount`.

use serde::{Deserialize, Serialize};

use super::paths::parse_iso_epoch;
use super::run::SyncRun;

/// What one write-drain pass did, as recorded on the mount.
///
/// Deliberately small and flat: this is an operator-facing receipt, not the
/// drain's working state. `pending` is the field that matters — it is the only
/// thing that distinguishes a mount which is caught up from one that is falling
/// behind, and neither `outcome: ok` nor a zero `failed` count says anything
/// about it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainSummary {
    /// Edits that reached the provider.
    #[serde(default)]
    pub pushed: u64,
    /// Candidates never attempted because the drain ended early.
    ///
    /// Non-zero only alongside `truncated` or `stopped`. These stay pending
    /// untouched — no stamp, no `__pushed_state` — so the next run re-nominates
    /// them in the same deterministic order.
    #[serde(default)]
    pub pending: u64,
    /// Pushes the provider answered with "no such object under that id".
    #[serde(default)]
    pub gone: u64,
    /// Local deletes propagated to the provider.
    #[serde(default)]
    pub deleted: u64,
    /// Local deletes deliberately NOT propagated, under `delete_policy: detach`.
    ///
    /// Reported separately from [`Self::deleted`] because the remote object is
    /// still there and will be re-imported by the next reconcile. Folding the
    /// two together would let a mount report deletions it never made.
    #[serde(default)]
    pub detached: u64,
    /// A blast-radius rail refused this run's deletes (§9.4). Everything is
    /// parked on [`MountState::writeback_pending_deletes`], nothing was sent,
    /// and reads were unaffected.
    #[serde(default)]
    pub blocked: bool,
    /// Pushes that failed outright.
    #[serde(default)]
    pub failed: u64,
    /// The wall-clock budget ran out with candidates still pending.
    #[serde(default)]
    pub truncated: bool,
    /// An operator's Stop landed mid-drain.
    #[serde(default)]
    pub stopped: bool,
    /// Updates withheld whole under `move_policy: reject`, because the node had
    /// been moved locally.
    ///
    /// Not a failure and not a skip: the mount is doing exactly what it was
    /// configured to do, and is also not writing the nodes it otherwise would.
    /// Both halves have to be visible or `outcome: ok` reads as "caught up".
    #[serde(default)]
    pub rejected: u64,
    /// Pushes the provider refused because the object had changed since this
    /// mount last read it, and the mount's conflict policy abandoned them.
    ///
    /// Counted apart from [`Self::failed`] because nothing is broken: the
    /// provider answered, and `remote_wins` — the default — deliberately drops
    /// the local edit and waits for the next delta to overwrite it. That is a
    /// LOST EDIT, though, and a mount losing edits every run must not report the
    /// same clean receipt as one with nothing to do.
    #[serde(default)]
    pub conflicts: u64,
    /// Conflicts left PENDING: `conflict: "error"`, a resolver that parked, a
    /// resolver that threw, or an answer this engine does not recognize.
    ///
    /// The difference from [`Self::conflicts`] is what the operator must do.
    /// A conflict is self-clearing; a parked one is not — it comes back on every
    /// drain until a human resolves it, and the mount sits at
    /// `writeback_status: "conflict"` meanwhile.
    #[serde(default)]
    pub parked: u64,

    // ---- `submit` outboxes only (§5) ----
    //
    // Skipped when zero so a mount that has never had an outbox does not carry
    // four zeroes in its state blob forever.
    /// Commands the provider accepted.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub submitted: u64,
    /// Commands returned to `queued` because the provider explicitly refused to
    /// act (`rate_limited`) — the only outcome on this path that is retried.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub requeued: u64,
    /// Commands terminally failed: definitively not sent.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub abandoned: u64,
    /// Commands parked at `unknown` — they may or may not have been issued.
    ///
    /// The one number on an outbox that always needs a human. It is reported
    /// apart from [`Self::abandoned`] because the two require opposite actions:
    /// a failed command can be requeued freely, an unresolved one must be
    /// checked at the provider FIRST or requeueing it sends a second copy.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unresolved: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// A local delete the reconcile walk found, queued until the drain acts on it.
///
/// Everything a push needs is captured HERE, at detection time, because
/// detection is the half that cannot be reconstructed later: once the watermark
/// has moved past a delete's revision and the MVCC pre-image it was recovered
/// from has been garbage collected, nothing in the system remembers that the
/// node existed — not its provider id, not its etag, not that this mount owned
/// it.
///
/// A queued delete is not a committed one. `write::guard` may refuse the whole
/// batch (§9.4), in which case these stay exactly as they are until an operator
/// confirms; nothing here is ever discarded to make a rail's arithmetic work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDelete {
    /// The node that was deleted.
    pub node_id: String,
    /// The provider-side id, recovered from the MVCC pre-image. Never empty —
    /// a delete without one is not provably this mount's to push (§9.3).
    pub external_id: String,
    /// The node's path at the moment before it was deleted, for the operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The `__etag` the pre-image carried, as the delete's optimistic-concurrency
    /// base.
    ///
    /// Captured at detection because there is nowhere else to get it: by the
    /// time anything drains, the node is gone and the pre-image may be
    /// collected. An adapter that honours it refuses to delete a remote object
    /// that has changed since this mount last saw it — which is the difference
    /// between propagating a delete and losing an edit someone else made in the
    /// window. `None` when the node was never etagged; the delete is then
    /// unconditional, as it has to be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The revision that deleted it, as an HLC string.
    pub revision: String,
    /// Unix epoch seconds at which the walk found it.
    pub detected_at: i64,
    /// The deleting transaction touched more nodes than
    /// [`BULK_REVISION_THRESHOLD`], so this is one of a mass delete.
    ///
    /// Free, and nothing else gives you the signal: `RevisionMeta.changed_nodes`
    /// says how wide the originating transaction was, which is the difference
    /// between a user deleting one message and a mis-scoped
    /// `DELETE FROM 'ws' WHERE path LIKE '/mail/%'`. Carried from detection so
    /// the rails can require confirmation on it regardless of how many deletes
    /// end up in any one drain.
    #[serde(default)]
    pub bulk: bool,
}

/// Above this many nodes in one revision, a delete is flagged `bulk`.
///
/// A deliberate low number: a person deleting things by hand does not commit
/// fifty nodes at once, and everything that does (a SQL DML statement, a
/// subtree prune, a function loop) is exactly what the confirmation rail exists
/// to catch.
pub const BULK_REVISION_THRESHOLD: usize = 50;

/// How many detected deletes a mount retains before the walk stops advancing.
///
/// Back-pressure, not truncation: on overflow the reconcile walk leaves the
/// watermark where it is and stops, so the deletes it has not recorded are
/// found again on the next pass. Dropping them instead would lose them
/// permanently — the walk is the only thing that can see a delete at all.
pub const MAX_PENDING_DELETES: usize = 500;

/// A tripped safety rail, recorded on the mount until an operator releases it.
///
/// The soft-block half of §9.4. Nothing is discarded when a rail trips: the
/// pending deletes stay exactly where they are on
/// [`MountState::writeback_pending_deletes`], and this records why they are not
/// moving and what to type to let them.
///
/// Modelled as one object rather than a `bool` beside a reason beside a token
/// because those three can disagree — a `writeback_blocked: true` with no reason
/// is a mount that has stopped writing and cannot say why, which is the silence
/// the block exists to break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritebackBlock {
    /// Fingerprint of the exact batch that was refused. An operator releases it
    /// by copying this into [`MountState::writeback_confirm_token`].
    ///
    /// Content-derived, so a confirmation authorises the batch that was
    /// reviewed and not whatever has accumulated since.
    pub token: String,
    /// The full operator-facing explanation, including how to release it.
    pub reason: String,
    /// How many deletes are parked.
    pub deletes: u64,
    /// Unix epoch seconds the rail tripped.
    pub at: i64,
}

/// What one reconcile walk did, as recorded on the mount.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileSummary {
    /// Unix epoch seconds the walk finished.
    #[serde(default)]
    pub at: i64,
    /// Revisions read from the watermark.
    #[serde(default)]
    pub scanned: u64,
    /// Revisions skipped because the sync engine itself wrote them.
    #[serde(default)]
    pub echoes: u64,
    /// Node changes on the mount's branch/workspace that were NOT the sync's
    /// own. An upper bound on local edits: ownership is not what the walk reads
    /// each node for (the drain re-checks divergence against the index anyway),
    /// so a change here may well belong to a node this mount has nothing to do
    /// with.
    #[serde(default)]
    pub local_changes: u64,
    /// Deletes newly recorded on [`MountState::writeback_pending_deletes`].
    #[serde(default)]
    pub deletes_found: u64,
    /// Mount-owned nodes found OUTSIDE the mount path and detached — the node
    /// keeps its content, loses its provenance, and the remote is untouched.
    ///
    /// Expect the provider's object to come back as a NEW node at the next
    /// reconcile: there is no per-mount suppression set. See `write::relocate`.
    #[serde(default)]
    pub detached: u64,
    /// Foreign nodes found INSIDE the mount path and refused adoption. Nothing
    /// was created at the provider and nothing was written locally.
    #[serde(default)]
    pub rejected: u64,
    /// The limit was reached with revisions still unread, or the pending-delete
    /// cap stopped the walk. Either way there is more to do and the next pass
    /// resumes from the watermark.
    #[serde(default)]
    pub truncated: bool,
}

/// Engine-managed `state` object of a `raisin:VirtualMount`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MountState {
    #[serde(default)]
    pub last_sync_token: Option<String>,
    /// Unix epoch seconds of the last successful sync write.
    #[serde(default)]
    pub last_sync_at: Option<i64>,
    /// Unix epoch seconds of the last sync ATTEMPT, successful or not.
    ///
    /// Separate from `last_sync_at` because the backoff has to be measured from
    /// the last try, not the last success. Scheduling on `last_sync_at` alone
    /// meant a mount that kept failing never advanced its clock, so
    /// `last_sync_at + effective_interval <= now` stayed true forever and the
    /// engine re-enqueued it on every 60s tick — the exponential backoff was
    /// dead in exactly the case it exists for, and a mount broken by a bad
    /// config hammered the provider indefinitely.
    #[serde(default)]
    pub last_attempt_at: Option<i64>,
    /// Resume point for an in-progress full walk ("backfill").
    ///
    /// `Some` means the last full pass stopped at `max_items_per_sync` without
    /// exhausting the provider. Without this the walk restarted from the top on
    /// every run, so a mailbox larger than the cap re-imported its first N items
    /// forever and never reached the rest — the reason a production-sized
    /// mailbox could not be imported at all.
    ///
    /// Holds the remaining folder stack as `(folder_id, path_prefix)` pairs plus
    /// the page cursor within the folder currently being walked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backfill_stack: Vec<(Option<String>, String)>,
    /// Items materialized so far by the CURRENT full walk / backfill / remap.
    ///
    /// Cumulative across the chunks of one walk and reset when a fresh walk
    /// starts, so the console can show "1,240 imported" while a multi-hour
    /// import is running instead of a spinner with no end in sight.
    ///
    /// **Backfill only.** The delta path used to add to this too, which meant a
    /// counter documented as "the current walk" accumulated incremental work
    /// forever — nothing in the delta path resets it, so a settled mount showed
    /// "433,500 items imported" and an import that could never finish. Delta
    /// work goes to [`Self::delta_items_done`].
    #[serde(default)]
    pub backfill_items_done: u64,
    /// Items the delta path has materialized since the last full walk.
    ///
    /// Separate from [`Self::backfill_items_done`] so incremental work stays
    /// visible — the reason it was folded in originally — without corrupting
    /// backfill progress. Reset alongside it when a fresh walk starts.
    #[serde(default)]
    pub delta_items_done: u64,
    /// Scheduling is suspended by an operator.
    ///
    /// Distinct from `MountConfig.enabled`: disabling a mount also tears down
    /// its push subscription (`mod.rs`, `teardown_push`), so notifications that
    /// arrive while it is off are lost. A pause keeps the subscription
    /// registered and simply stops the scheduler, so resuming is instant and
    /// nothing is missed in the gap.
    ///
    /// Lives in `state` rather than as a node property deliberately: the
    /// nodetype is `strict: true`, but `state` is a free-form Object, so this
    /// needs no schema migration.
    #[serde(default)]
    pub paused: bool,
    /// An operator asked the running sync to stop.
    ///
    /// Cooperative, never a hard abort. Aborting the task would skip the lease
    /// release and leave the mount locked for the full `SYNC_LEASE_TTL`, with
    /// `status` stuck at `"syncing"` — a phantom run the console cannot clear.
    /// The page loops check this where they already flush and persist, so a stop
    /// always lands on a clean boundary.
    #[serde(default)]
    pub stop_requested: bool,
    /// True once a full walk has reached the end at least once. Reconcile
    /// deletes are only safe after this: before it, "not seen" merely means
    /// "not reached yet".
    #[serde(default)]
    pub backfill_complete: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u64,
    /// `"ok" | "syncing" | "auth_required" | "degraded" | "misconfigured"`.
    #[serde(default)]
    pub status: Option<String>,
    /// Total items the current walk expects to materialize, when the provider
    /// reports it. The denominator for "37 of 500"; `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill_items_total: Option<u64>,
    /// Durable, node-resident write counter — the concurrency guard for this
    /// state blob.
    ///
    /// This deliberately does NOT come from the lock manager. A lock is for
    /// locking, not for storing data: `raisin_locks`' fencing counter lives in
    /// process memory (`inprocess`) or in a plain un-TTL'd Redis key, and both
    /// reset to zero on restart while this blob survives in RocksDB. Comparing
    /// the two rejected every write after a restart until the volatile counter
    /// climbed back past the stored one — silently, because every caller
    /// discarded the result. A sync would materialize hundreds of nodes and
    /// then throw its own cursor away.
    ///
    /// So the guard is durable and lives with the data it guards, exactly like
    /// `FlowInstance.version`: a writer carries the seq it read, and
    /// [`crate::jobs::handlers::virtual_mount_sync::persist_mount_state`] refuses the write if the stored seq has
    /// moved since. Wiping the lock store now costs duplicate work at worst,
    /// never a discarded write.
    #[serde(default)]
    pub state_seq: u64,
    /// Whether this mount's requested writeback can actually be honoured.
    ///
    /// Computed once per run from the adapter's declared capabilities (see
    /// [`crate::jobs::handlers::virtual_mount_sync::Capabilities::missing_mirror_ops`]) AND from a probe of the mount's
    /// mapper (see [`crate::jobs::handlers::virtual_mount_sync::MapperWriteback`]) — writability belongs to the mount,
    /// adapter and mapper together, so a write-capable adapter behind a
    /// read-only mapper is honestly reported here as not writable. Never
    /// hardcoded, so the console
    /// shows what is true rather than what v1 happened to implement. `None` when
    /// the mount never asked for writeback: absent and `Some(false)` are NOT
    /// interchangeable to the UI, which reads absence as "unknown / not
    /// applicable" and a present `false` as an authoritative "cannot". Never
    /// makes the sync fatal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_supported: Option<bool>,
    /// Health of the OUTBOUND half of this mount, distinct from
    /// [`Self::status`] (which is about the read side).
    ///
    /// `"conflict"` means at least one edit is parked awaiting a human: the
    /// provider refused it and the mount's policy — `error`, or a resolver that
    /// parked/threw/answered something unrecognized — declined to decide.
    /// `"misconfigured"` means the drain never started because `write_config`
    /// could not be resolved. Cleared by the first drain that parks nothing.
    ///
    /// A separate field rather than another `writeback_last_error` string
    /// because the console has to be able to BADGE this: a message that a human
    /// must read to learn the mount is stuck is the same silence as no message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_status: Option<String>,
    /// Why [`Self::writeback_supported`] is `Some(false)` — e.g. which adapter
    /// operation is missing. Cleared when writeback is supported.
    ///
    /// Lives in `state` rather than as a node property for the same reason as
    /// [`Self::paused`]: the nodetype is `strict: true`, but `state` is a
    /// free-form Object, so this needs no schema migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_last_error: Option<String>,
    /// What the write drain did on the most recent run.
    ///
    /// The drain's own counters are otherwise consumed entirely inside
    /// `write::drain` and dropped, which makes a drain that gave up at 120 of
    /// 500 edits indistinguishable — to an operator, and to the console — from
    /// one that had nothing to do. That is the same class of invisibility as a
    /// sync that never records why it stopped.
    ///
    /// Absent on a mount that has never drained. Written on EVERY drain that had
    /// candidates, including a fully clean one, so a stale summary can never be
    /// mistaken for the current state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_drain: Option<DrainSummary>,

    // ---- durable change detection (the reconcile watermark) ----
    /// How far the [`RevisionMeta`](raisin_storage::RevisionMeta) walk has read,
    /// as an HLC string.
    ///
    /// The durable half of change detection. An event-bus hook is fast and
    /// in-process; this is what survives a crash, a restart and a failover, and
    /// it is the ONLY thing that can see a delete — a deleted node is gone from
    /// the workspace, absent from the sync index, and indistinguishable from a
    /// node that was never there.
    ///
    /// `None` means the mount has never reconciled. It is then initialized to
    /// the repository's CURRENT head rather than to zero: walking a repo's
    /// entire history the first time writeback is enabled would spend an
    /// unbounded amount of work re-deriving edits the drain's divergence check
    /// already covers, and would surface deletes from before the mount could
    /// have pushed them.
    ///
    /// Lives in `state` rather than as a node property for the same reason as
    /// [`Self::paused`]: the nodetype is `strict: true`, `state` is free-form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_revision: Option<String>,
    /// Deletes the walk has found and nothing has acted on yet.
    ///
    /// Bounded by [`MAX_PENDING_DELETES`]; see that constant for why the walk
    /// stops rather than truncating.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writeback_pending_deletes: Vec<PendingDelete>,
    /// What the most recent reconcile walk did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reconcile: Option<ReconcileSummary>,
    /// A safety rail has refused this mount's pending deletes (§9.4).
    ///
    /// Present means outbound writes are PARKED — not discarded. Reads are
    /// deliberately unaffected: refusing to destroy remote data must never also
    /// stop new data arriving, or the pragmatic response to a scare becomes
    /// "turn the rail off".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_blocked: Option<WritebackBlock>,
    /// The operator's release: set it to the blocked batch's
    /// [`WritebackBlock::token`] and the next drain pushes exactly that batch.
    ///
    /// Consumed on use, and matched against a token derived from the batch's
    /// CONTENT, so a confirmation given for three deletes cannot silently
    /// authorise thirty that arrived after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_confirm_token: Option<String>,

    // ---- push / webhook subscription (all engine-managed, optional) ----
    /// Stable, unguessable per-mount token that gates the public notifications
    /// endpoint. Format: `"{mount_id}.{nanoid}"`. Generated once by the engine
    /// (see [`crate::jobs::handlers::virtual_mount_sync::subscription::ensure_mount_token`]) and embedded in the
    /// `notification_url` handed to the provider on `subscribe`. The HTTP
    /// notifications endpoint parses `mount_id` from the prefix, loads this
    /// mount, and constant-time compares the incoming token against this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_mount_token: Option<String>,
    /// Provider-side subscription/channel id returned by the adapter's
    /// `subscribe` op. Its presence (with a future `push_expires_at`) marks the
    /// mount as having a live push subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_subscription_id: Option<String>,
    /// Optional shared secret returned by `subscribe` (e.g. Graph `clientState`,
    /// Google channel token). The notifications endpoint MAY verify it when the
    /// provider echoes it; the `push_mount_token` in the URL is the primary auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_secret: Option<String>,
    /// ISO-8601 expiry of the provider subscription. The renewal job refreshes
    /// subscriptions whose expiry is within one day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_expires_at: Option<String>,
    /// The `notification_url` last handed to the provider (for renew/debug).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notification_url: Option<String>,
    /// `"active" | "failed" | "unsupported"` (or `None` when never attempted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_status: Option<String>,
    /// Last push-subscription error message, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_last_error: Option<String>,

    // ---- webhook DELIVERY health (distinct from subscription health) ----
    //
    // A live subscription says the provider accepted our URL once. It says
    // nothing about whether its notifications are getting through: a mount can
    // sit at `push_status: "active"` while every delivery is rejected, which is
    // exactly how a production import stalled for hours with a green badge.
    /// Unix epoch seconds of the last notification the endpoint ACCEPTED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_last_delivery_at: Option<i64>,
    /// Unix epoch seconds of the last notification the endpoint REJECTED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_last_rejected_at: Option<i64>,
    /// Why the last rejected delivery was rejected (e.g. `"secret mismatch"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_last_rejected_reason: Option<String>,
    /// Deliveries accepted / rejected since the mount was created.
    #[serde(default)]
    pub push_deliveries_ok: u64,
    #[serde(default)]
    pub push_deliveries_rejected: u64,

    // ---- per-run reporting ----
    /// The most recent sync run, terminal or in flight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<SyncRun>,
    /// Bounded history, newest first, capped at [`MAX_RUN_HISTORY`].
    ///
    /// Bounded on purpose: a mount syncing every five minutes would otherwise
    /// grow this blob without limit, and it is rewritten on every progress tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_runs: Vec<SyncRun>,
}

/// How many terminal runs [`MountState::recent_runs`] retains.
pub const MAX_RUN_HISTORY: usize = 20;

impl MountState {
    /// Whether this mount currently has a usable push subscription: an id is
    /// present, status is `active`, and the expiry (if known) is in the future.
    /// `now` is Unix epoch seconds.
    pub fn has_active_push(&self, now: i64) -> bool {
        if self.push_status.as_deref() != Some("active") {
            return false;
        }
        if self.push_subscription_id.is_none() {
            return false;
        }
        match &self.push_expires_at {
            None => true,
            Some(iso) => parse_iso_epoch(iso).map(|t| t > now).unwrap_or(true),
        }
    }

    /// Whether the push subscription expires within `window_secs` of `now`
    /// (Unix seconds) and should be renewed. Only meaningful for active subs.
    pub fn push_expires_within(&self, now: i64, window_secs: i64) -> bool {
        match self.push_expires_at.as_deref().and_then(parse_iso_epoch) {
            Some(exp) => exp <= now + window_secs,
            None => false,
        }
    }
}
