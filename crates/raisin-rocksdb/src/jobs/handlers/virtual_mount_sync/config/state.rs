//! The engine-managed `state` blob of a `raisin:VirtualMount`.

use serde::{Deserialize, Serialize};

use super::paths::parse_iso_epoch;
use super::run::SyncRun;

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
    /// `false` when the mount requests `write_through` writeback but the engine
    /// or adapter cannot honour it (v1 has no writeback implementation). `None`
    /// when writeback was never requested. The UI reads this to explain the
    /// limitation; it never makes the sync fatal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writeback_supported: Option<bool>,

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
