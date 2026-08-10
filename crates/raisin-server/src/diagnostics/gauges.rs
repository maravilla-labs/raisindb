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

//! Sizes of the long-lived in-process collections.
//!
//! [`super::allocator`] says *whether* the Rust heap is growing;
//! these gauges say *which collection*. Every entry here is a map or list that
//! outlives a request and whose key space is unbounded (per connection, per
//! flow instance, per conversation, per upload) — the shape that produces a
//! steady climb on an otherwise idle server.
//!
//! **A number that only ever goes up is the finding.** Absolute values matter
//! less than the slope across samples, which is why these are cheap enough to
//! poll on a monitoring interval.

use serde::Serialize;

/// A sample of every tracked in-process collection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Gauges {
    /// Registered job monitors on the storage-owned registry — where the job
    /// SSE stream registers in a RocksDB build. Should sit at a small constant:
    /// the dispatcher plus one per *currently open* stream. A climbing value
    /// means a registration site is not holding its `JobMonitorGuard` — the
    /// leak this endpoint was built to catch.
    pub job_monitors: usize,
    /// Registered job monitors on the process-wide registry.
    pub global_job_monitors: usize,

    /// Job entries in the storage-owned registry. Trimmed every 60s by the
    /// cleanup sweep, so this should oscillate rather than climb.
    pub job_registry_jobs: usize,
    /// Job entries in the process-wide `global_registry`. **Nothing sweeps
    /// this one.** Admin-triggered integrity scans, backups, maintenance runs
    /// and vector reindexes register here and are never removed, so a climbing
    /// value is expected — the question is how fast.
    pub global_job_registry_jobs: usize,

    /// Broadcast channels for flow instances. Released when the last
    /// subscription drops; a climb means a subscribe site is missing its
    /// `FlowEventSubscription`. Each retained channel pins up to 100 step
    /// outputs.
    pub flow_event_channels: usize,
    /// Broadcast channels for AI conversations. Same contract, larger payloads
    /// — streamed text chunks and tool-call result JSON.
    pub conversation_event_channels: usize,
    /// Broadcast channels for virtual-mount syncs. Already guarded; included
    /// as the control — if this one climbs too, the problem is not the guards.
    pub mount_event_channels: usize,

    /// Upload sessions held in memory. Monotonic by construction today
    /// (nothing calls `delete`, nothing enforces `expires_at`).
    pub upload_sessions: usize,

    /// Function module cache: total entries.
    pub module_cache_entries: usize,
    /// Function module cache entries past their TTL but still resident.
    /// `TtlCache::get` returns `None` for these while the map keeps the module
    /// source text, and `cleanup_expired` has no production caller — so this
    /// number is pure retained garbage.
    pub module_cache_expired_entries: usize,
}

/// Read every gauge. Cheap: each is a `len()` behind a lock or a `DashMap`.
///
/// `storage_registry` is the backend-owned [`JobRegistry`]; pass it so both it
/// and the never-swept global registry are sampled. The two are genuinely
/// different objects with different lifecycles, and reading only one has
/// already misled us once.
pub async fn sample(storage_registry: Option<&raisin_storage::jobs::JobRegistry>) -> Gauges {
    let global_registry = raisin_storage::jobs::global_registry();
    let module_cache = raisin_functions::execution::module_cache::module_cache_stats();

    let (job_monitors, job_registry_jobs) = match storage_registry {
        Some(registry) => (registry.monitors().len().await, registry.job_count().await),
        None => (0, 0),
    };

    Gauges {
        job_monitors,
        global_job_monitors: global_registry.monitors().len().await,
        job_registry_jobs,
        global_job_registry_jobs: global_registry.job_count().await,

        flow_event_channels: raisin_storage::jobs::global_flow_broadcaster().channel_count(),
        conversation_event_channels: raisin_storage::jobs::global_conversation_broadcaster()
            .channel_count(),
        mount_event_channels: raisin_storage::jobs::global_mount_broadcaster().channel_count(),

        upload_sessions: raisin_transport_http::upload_session_count().await,

        module_cache_entries: module_cache.total_entries,
        module_cache_expired_entries: module_cache.expired_entries,
    }
}
