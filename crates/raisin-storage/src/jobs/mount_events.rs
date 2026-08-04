// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Live progress for a virtual mount's sync.
//!
//! Modelled directly on [`flow_events`](super::flow_events): one broadcaster,
//! keyed per entity, consumed by whichever transport wants it. That crate proved
//! the shape — WS and SSE both read the same `FlowEventBroadcaster` — and the
//! same applies here.
//!
//! **Why a dedicated stream rather than node events.** The console used to infer
//! progress from `node:updated` on the mount's config node, which says only
//! "something changed": no counts, no phase, and one event per state write
//! whether or not anything was imported. It also required a WebSocket, the one
//! transport that cannot carry an `Authorization` header — the reason that page
//! was the only one in the console with an auth problem.
//!
//! Events are emitted where the sync **already** flushes its batch and persists
//! its cursor. That is deliberate: an event from any other point would claim
//! progress that a later failure could still roll back.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Global broadcaster, shared by every sync run and every SSE client.
static GLOBAL_MOUNT_BROADCASTER: Lazy<MountSyncBroadcaster> = Lazy::new(MountSyncBroadcaster::new);

/// The process-wide mount sync broadcaster.
pub fn global_mount_broadcaster() -> &'static MountSyncBroadcaster {
    &GLOBAL_MOUNT_BROADCASTER
}

/// Events buffered per mount before a slow subscriber starts lagging.
///
/// Small on purpose. A subscriber that falls behind wants the *current* state,
/// not a replay of every page — and the payload is cumulative, so the newest
/// event alone is enough to render the card correctly.
const CHANNEL_CAPACITY: usize = 32;

/// A progress event from a running sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MountSyncEvent {
    /// A run began.
    Started {
        /// The phase actually about to execute — `delta`, `full` or `remap`.
        ///
        /// The phase, NOT the requested mode. A run asked for `delta` can
        /// execute a full reconcile when the provider rejects a stale cursor,
        /// and reporting the request instead of the reality is what left an
        /// operator watching runs labelled "Delta" write five-year-old mail.
        phase: String,
        /// What scheduled it: `schedule`, `manual`, `push`.
        trigger: String,
    },
    /// A page landed: batch flushed, cursor durable.
    Progress {
        phase: String,
        /// Items materialized by the current full walk.
        backfill_items_done: u64,
        /// Items materialized by the delta path since the last full walk.
        delta_items_done: u64,
        /// Denominator, when the provider reports one. Usually absent.
        items_total: Option<u64>,
        /// Folders still queued for the walk. Non-zero means "still going",
        /// even on a page that wrote nothing.
        subtrees_queued: usize,
    },
    /// The run ended.
    Finished {
        phase: String,
        /// `ok`, `error`, `cancelled`, `auth_required`, `skipped`…
        outcome: String,
        written: u64,
        skipped: u64,
        deleted: u64,
        failed: u64,
        /// Whether a full walk has now reached the end at least once.
        backfill_complete: bool,
        error: Option<String>,
    },
}

impl MountSyncEvent {
    /// Stable name for logs and the SSE `event:` field.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Started { .. } => "started",
            Self::Progress { .. } => "progress",
            Self::Finished { .. } => "finished",
        }
    }
}

/// Per-mount broadcast channels.
pub struct MountSyncBroadcaster {
    channels: Arc<DashMap<String, broadcast::Sender<MountSyncEvent>>>,
}

impl Default for MountSyncBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl MountSyncBroadcaster {
    /// Create an empty broadcaster.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Subscribe to one mount's progress, creating the channel if needed.
    pub fn subscribe(&self, mount_id: &str) -> broadcast::Receiver<MountSyncEvent> {
        let sender = self.channels.entry(mount_id.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            tx
        });
        sender.subscribe()
    }

    /// Emit an event for a mount.
    ///
    /// A mount with no subscribers drops the event, which is the normal case —
    /// syncs run on a timer whether or not anyone is watching. Emitting must
    /// therefore never be on the sync's critical path, and never fail it.
    pub fn emit(&self, mount_id: &str, event: MountSyncEvent) {
        if let Some(sender) = self.channels.get(mount_id) {
            let _ = sender.send(event);
        }
    }

    /// Drop a mount's channel once nobody is listening.
    ///
    /// Without this the map grows by one entry per mount ever synced and never
    /// shrinks — small, but unbounded, and this process runs for months.
    pub fn release_if_idle(&self, mount_id: &str) {
        self.channels
            .remove_if(mount_id, |_, sender| sender.receiver_count() == 0);
    }

    /// Live channel count. Test/diagnostic only.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(done: u64) -> MountSyncEvent {
        MountSyncEvent::Progress {
            phase: "full".to_string(),
            backfill_items_done: done,
            delta_items_done: 0,
            items_total: None,
            subtrees_queued: 1,
        }
    }

    #[tokio::test]
    async fn a_subscriber_receives_progress() {
        let b = MountSyncBroadcaster::new();
        let mut rx = b.subscribe("mount-1");
        b.emit("mount-1", progress(600));

        match rx.recv().await.expect("an event") {
            MountSyncEvent::Progress {
                backfill_items_done,
                subtrees_queued,
                ..
            } => {
                assert_eq!(backfill_items_done, 600);
                assert_eq!(subtrees_queued, 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Emitting with nobody listening must be harmless — syncs run on a timer
    /// whether or not the console is open, and progress reporting must never be
    /// able to fail a sync.
    #[test]
    fn emitting_without_subscribers_is_a_no_op() {
        let b = MountSyncBroadcaster::new();
        b.emit("nobody-home", progress(1));
        assert_eq!(b.channel_count(), 0);
    }

    /// Channels are per mount, so one mount's progress never reaches another's
    /// watcher.
    #[tokio::test]
    async fn channels_do_not_leak_across_mounts() {
        let b = MountSyncBroadcaster::new();
        let mut rx = b.subscribe("mount-1");
        b.emit("mount-2", progress(5));
        b.emit("mount-1", progress(7));

        match rx.recv().await.unwrap() {
            MountSyncEvent::Progress {
                backfill_items_done,
                ..
            } => assert_eq!(backfill_items_done, 7, "must not see mount-2's event"),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The map must not grow by one entry per mount ever synced.
    #[test]
    fn an_idle_channel_is_released() {
        let b = MountSyncBroadcaster::new();
        {
            let _rx = b.subscribe("mount-1");
            assert_eq!(b.channel_count(), 1);
            // A live receiver keeps it.
            b.release_if_idle("mount-1");
            assert_eq!(b.channel_count(), 1);
        }
        // Receiver dropped: now it goes.
        b.release_if_idle("mount-1");
        assert_eq!(b.channel_count(), 0);
    }
}
