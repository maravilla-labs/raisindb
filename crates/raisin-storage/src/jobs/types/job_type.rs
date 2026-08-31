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

//! JobType enum definition

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

use super::asset_processing::AssetProcessingOptions;
use super::index_operation::IndexOperation;

/// Type of background job
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum JobType {
    IntegrityScan,
    IndexRebuild,
    IndexVerify,
    Compaction,
    Backup,
    Restore,
    OrphanCleanup,
    Repair,
    TreeSnapshot {
        revision: HLC,
    },
    FulltextVerify,
    FulltextRebuild,
    FulltextOptimize,
    FulltextPurge,
    VectorVerify,
    VectorRebuild,
    VectorOptimize,
    VectorRestore,
    FulltextIndex {
        node_id: String,
        operation: IndexOperation,
    },
    FulltextBranchCopy {
        source_branch: String,
    },
    FulltextBatchIndex {
        operation_count: usize,
    },
    EmbeddingGenerate {
        node_id: String,
    },
    EmbeddingDelete {
        node_id: String,
    },
    EmbeddingBranchCopy {
        source_branch: String,
    },
    HuggingFaceModelDownload {
        model_id: String,
    },
    HuggingFaceModelDelete {
        model_id: String,
    },
    AssetProcessing {
        node_id: String,
        options: AssetProcessingOptions,
    },
    ReplicationGC {
        tenant_id: String,
        repo_id: String,
    },
    ReplicationSync {
        tenant_id: String,
        repo_id: String,
        peer_id: Option<String>,
    },
    OpLogCompaction {
        tenant_id: String,
        repo_id: String,
    },
    PropertyIndexBuild {
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
    },
    CompoundIndexBuild {
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
        node_type_name: String,
        index_name: String,
    },
    /// Build or rebuild the geohash spatial index for a workspace.
    ///
    /// This is the backfill for data written before spatial indexing existed (or
    /// before the current precision policy), and the reconciliation step after a
    /// `policy_hash` change. Each cluster node runs its own build against its own
    /// local index — the spatial index is derived local state, exactly like the
    /// Tantivy fulltext index, so there is nothing to coordinate.
    SpatialIndexBuild {
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
        /// `None` builds every geometry-valued property in the workspace.
        property: Option<String>,
        /// `true` re-emits every entry and tombstones superseded ones; `false`
        /// only fills gaps (nodes not already covered by the current policy hash).
        rebuild: bool,
    },
    BulkSql {
        sql: String,
        actor: String,
    },
    RevisionHistoryCopy {
        source_branch: String,
        target_branch: String,
        up_to_revision: HLC,
    },
    CopyTree {
        source_id: String,
        target_parent_id: String,
        new_name: Option<String>,
        recursive: bool,
    },
    RestoreTree {
        node_id: String,
        node_path: String,
        revision_hlc: String,
        recursive: bool,
        translations: Option<Vec<String>>,
    },
    NodeDeleteCleanup {
        node_id: String,
        workspace: String,
    },
    /// Retarget the denormalized `path` in reference-index forward entries that
    /// point at a moved node, so incoming references reflect its NEW path.
    /// Enqueued (per moved node) when a node move emits its `Updated` event.
    RetargetReferences {
        /// The moved (target) node id.
        node_id: String,
        /// The moved node's workspace (the reference target workspace).
        workspace: String,
        /// The moved node's new path (authoritative).
        new_path: String,
    },
    RelationConsistencyCheck {
        repair: bool,
    },
    FunctionExecution {
        function_path: String,
        trigger_name: Option<String>,
        execution_id: String,
    },
    TriggerEvaluation {
        event_type: String,
        node_id: String,
        node_type: String,
    },
    ScheduledTriggerCheck {
        tenant_id: Option<String>,
        repo_id: Option<String>,
    },
    /// One-shot scheduled invocation of a function or flow at a fixed time.
    ///
    /// The variant is intentionally minimal (JobType round-trips through its
    /// Display string): the rich payload — target path, input, actor,
    /// external key, scheduled time — lives in the persisted `JobContext`.
    ScheduledInvocation {
        /// Unique invocation id (nanoid, slash-free).
        invocation_id: String,
        /// Invocation target kind: "function" or "flow".
        target_kind: String,
    },
    FlowExecution {
        flow_execution_id: String,
        trigger_path: String,
        flow: serde_json::Value,
        current_step_index: usize,
        step_results: serde_json::Value,
    },
    FlowInstanceExecution {
        instance_id: String,
        execution_type: String,
        resume_reason: Option<String>,
    },
    AICall {
        instance_id: String,
        step_id: String,
        agent_ref: String,
        iteration: u32,
    },
    PackageInstall {
        package_name: String,
        package_version: String,
        package_node_id: String,
    },
    PackageProcess {
        package_node_id: String,
    },
    PackageExport {
        package_name: String,
        package_node_id: String,
        export_mode: String,
        include_modifications: bool,
    },
    PackageSyncStatus {
        package_node_id: String,
        compute_hashes: bool,
    },
    PackageSyncPush {
        package_node_id: String,
        paths_to_sync: Vec<String>,
    },
    PackageSyncPull {
        package_node_id: String,
        paths_to_pull: Vec<String>,
        conflict_resolution: String,
    },
    PackageCreateFromSelection {
        package_name: String,
        package_version: String,
        include_node_types: bool,
    },
    AIToolCallExecution {
        tool_call_path: String,
        tool_call_workspace: String,
    },
    AIToolResultAggregation {
        single_result_path: String,
        workspace: String,
    },
    AuthMagicLinkSend {
        identity_id: String,
        email: String,
        token_id: String,
    },
    AuthSessionCleanup {
        tenant_id: Option<String>,
        batch_size: usize,
    },
    AuthTokenCleanup {
        tenant_id: Option<String>,
        token_types: Vec<String>,
    },
    AuthAccessNotification {
        identity_id: String,
        repo_id: String,
        notification_type: String,
    },
    AuthCreateUserNode {
        identity_id: String,
        repo_id: String,
        email: String,
        display_name: Option<String>,
        default_roles: Vec<String>,
    },
    ResumableUploadComplete {
        upload_id: String,
        commit_message: Option<String>,
        commit_actor: Option<String>,
    },
    UploadSessionCleanup {
        upload_id: String,
    },
    /// Periodic scan: find due virtual mounts and enqueue per-mount sync jobs
    /// (mirrors `ScheduledTriggerCheck`).
    VirtualMountSyncCheck {
        tenant_id: Option<String>,
        repo_id: Option<String>,
    },
    // NOTE: `default_sync_trigger` is defined below the enum.
    /// Sync a single virtual mount. `mode` is "delta", "full" or "remap".
    VirtualMountSync {
        mount_id: String,
        mode: String,
        /// What caused this run: `"push" | "schedule" | "manual" | "unknown"`.
        ///
        /// Recorded on the mount's run history. Without it you cannot tell a
        /// mount that nothing is scheduling from one whose provider has gone
        /// quiet — the two look identical from the outside, which is precisely
        /// how a stalled import went undiagnosed.
        ///
        /// `#[serde(default)]` so jobs already queued before this field existed
        /// still deserialize.
        #[serde(default = "default_sync_trigger")]
        trigger: String,
    },
    /// Drain one virtual mount's pending local edits to its provider, NOW.
    ///
    /// The latency half of the write path. Correctness never depends on it:
    /// the drain also runs as the first phase of every ordinary
    /// [`Self::VirtualMountSync`], and [`Self::VirtualMountWriteReconcile`] is
    /// the durable change feed. This job only shortens the delay between an
    /// edit and its push from "the next poll interval" to "seconds".
    ///
    /// It takes the SAME per-mount lease a sync run takes and no-ops on
    /// contention — a sync already in flight will drain these edits itself, so
    /// waiting for the lease would only queue a second, redundant pass. That is
    /// also why per-process job dedup is enough here: the lease, not the dedup
    /// key, is the cluster-wide gate.
    VirtualMountWriteDrain {
        mount_id: String,
        /// What caused this drain: `"capture" | "manual" | "unknown"`.
        #[serde(default = "default_sync_trigger")]
        trigger: String,
    },
    /// Refresh expiring OAuth tokens across all integrations.
    IntegrationTokenRefresh {
        tenant_id: Option<String>,
    },
    /// Discover one external MCP server's tools and reconcile its proxy
    /// `raisin:Function` nodes.
    ///
    /// Carries the tenant/repo explicitly rather than relying on `JobContext`,
    /// mirroring `IntegrationTokenRefresh`, because the periodic scan enqueues
    /// it for repos it discovered rather than for the caller's own scope.
    McpToolDiscovery {
        connection_slug: String,
        tenant_id: String,
        repo_id: String,
        /// What asked for this run.
        ///
        /// Purely diagnostic, and the reason it is worth carrying: an operator
        /// looking at the job list cannot otherwise tell a server that
        /// announced a change from one being polled on its interval, which is
        /// exactly the question when someone asks "is the live update working?"
        ///
        /// `#[serde(default)]` so jobs queued before this field existed still
        /// deserialize.
        #[serde(default)]
        source: McpDiscoverySource,
    },
    /// Periodic scan: enqueue `McpToolDiscovery` for every enabled connection
    /// whose refresh interval is due (mirrors `VirtualMountSyncCheck`).
    McpDiscoveryCheck {
        tenant_id: Option<String>,
    },
    /// Periodic scan: walk each writable virtual mount's revisions from its
    /// writeback watermark, so local edits — and above all local DELETES — are
    /// detected durably rather than only while a process happens to be up.
    ///
    /// Mirrors [`Self::VirtualMountSyncCheck`], with one difference that shapes
    /// the handler: the sync check only enqueues, while this one walks and
    /// writes mount state. Job dedup is per-process, so the handler takes a
    /// per-mount `raisin_locks` lease; without it every node of a cluster would
    /// advance the same watermark and discard each other's findings.
    VirtualMountWriteReconcile {
        tenant_id: Option<String>,
        repo_id: Option<String>,
    },
    /// Periodic scan: rebuild the derived calendar-occurrence projection.
    ///
    /// A recurring event is stored as ONE series master carrying an RFC 5545
    /// rule, so `WHERE start_utc >= ? AND start_utc < ?` cannot see it — the
    /// master's own start is its first occurrence. This job expands each master
    /// over a rolling window into concrete, indexed occurrence nodes under
    /// `/_occurrences`.
    ///
    /// Derived, never authoritative: the projection is safe to delete and is
    /// rebuilt idempotently. Job dedup is per-PROCESS, so the handler takes a
    /// per-workspace `raisin_locks` lease whose TTL doubles as the rebuild
    /// interval.
    CalendarOccurrenceRebuild {
        tenant_id: Option<String>,
        repo_id: Option<String>,
    },
    /// Periodic scan: renew push/webhook subscriptions on virtual mounts whose
    /// provider subscription is close to expiring (mirrors
    /// `IntegrationTokenRefresh`).
    VirtualMountSubscriptionRenew {
        tenant_id: Option<String>,
    },
    Custom(String),
}

/// Serde default for [`JobType::VirtualMountSync::trigger`] — see that field.
fn default_sync_trigger() -> String {
    "unknown".to_string()
}

/// What caused an [`JobType::McpToolDiscovery`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpDiscoverySource {
    /// The remote server pushed `notifications/tools/list_changed`.
    Notification,
    /// The connection's refresh interval came due.
    Interval,
    /// An operator pressed refresh, or toggled a tool.
    Manual,
    /// Queued before this field existed, or by a path that did not say.
    #[default]
    Unknown,
}

impl McpDiscoverySource {
    /// Stable lowercase name, for log lines and job summaries.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Notification => "notification",
            Self::Interval => "interval",
            Self::Manual => "manual",
            Self::Unknown => "unknown",
        }
    }
}

impl JobType {
    /// Returns an appropriate timeout in seconds based on the job type.
    ///
    /// Quick jobs (triggers, aggregations) get shorter timeouts so the watchdog
    /// detects stuck ones faster. Function/AI executions get longer timeouts
    /// to accommodate AI API calls. Long-running maintenance jobs get the longest.
    pub fn default_timeout_seconds(&self) -> u64 {
        match self {
            // Quick jobs — should complete in under a minute
            JobType::TriggerEvaluation { .. } => 120,
            JobType::AIToolResultAggregation { .. } => 120,
            JobType::ScheduledTriggerCheck { .. } => 120,
            JobType::AuthSessionCleanup { .. } => 120,
            JobType::AuthTokenCleanup { .. } => 120,
            JobType::AuthMagicLinkSend { .. } => 120,
            JobType::AuthAccessNotification { .. } => 120,
            JobType::NodeDeleteCleanup { .. } => 120,
            JobType::RetargetReferences { .. } => 120,
            JobType::UploadSessionCleanup { .. } => 120,
            JobType::VirtualMountSyncCheck { .. } => 60,
            // A bounded revision walk per mount plus one MVCC read per delete.
            // No provider calls at all, so it has no business taking as long as
            // a sync.
            JobType::VirtualMountWriteReconcile { .. } => 120,
            // Pure local work: one type-index scan per workspace plus an
            // in-memory expansion. No provider calls at all.
            JobType::CalendarOccurrenceRebuild { .. } => 120,
            JobType::IntegrationTokenRefresh { .. } => 60,
            JobType::McpDiscoveryCheck { .. } => 60,
            // One handshake plus a paginated tools/list against a third party.
            JobType::McpToolDiscovery { .. } => 120,
            JobType::VirtualMountSubscriptionRenew { .. } => 120,
            // Long-running: syncing one mount can page through many remote items
            JobType::VirtualMountSync { .. } => 600,
            // The same provider calls a sync's drain phase makes, under the same
            // lease and the same wall-clock budget — so it gets the same timeout.
            // A shorter one would have the watchdog abort a legitimate drain
            // mid-push, which is the one thing the drain must never be.
            JobType::VirtualMountWriteDrain { .. } => 600,
            // Function/AI execution — can involve AI API calls (10-30s each)
            JobType::FunctionExecution { .. } => 300,
            JobType::ScheduledInvocation { .. } => 300,
            JobType::AIToolCallExecution { .. } => 300,
            JobType::AICall { .. } => 300,
            JobType::FlowExecution { .. } => 300,
            JobType::FlowInstanceExecution { .. } => 300,
            // Long-running background/maintenance jobs
            JobType::IntegrityScan => 600,
            JobType::IndexRebuild => 600,
            JobType::IndexVerify => 600,
            JobType::FulltextRebuild => 600,
            JobType::FulltextVerify => 600,
            JobType::FulltextPurge => 600,
            // The documented mitigation for a Tantivy merge storm, and the one
            // that runs when the index is at its worst: force-merging hundreds
            // of segments over a multi-GB index is minutes of work, not the
            // catch-all 300s. Under-budgeting it here would have the watchdog
            // abort the very operation an operator started to end an incident.
            JobType::FulltextOptimize => 1800,
            JobType::VectorRebuild => 600,
            // A full-workspace spatial backfill pages through every node; it
            // checkpoints its resume cursor so a timeout resumes rather than
            // restarts.
            JobType::SpatialIndexBuild { .. } => 600,
            JobType::Backup => 600,
            JobType::Restore => 600,
            // A package install is one job that writes hundreds of content
            // nodes plus their binaries in 100-node batches, and it neither
            // checkpoints nor resumes: a timed-out attempt is RESTARTED from
            // the top. Under the catch-all 300s the studio package (433
            // content nodes, 177 binaries) timed out on a slow box, and the
            // retry then ran CONCURRENTLY with the still-live first attempt
            // (the abort does not land inside a batch), so every create the
            // first attempt still held a path reservation for was rejected
            // with "being created by a concurrent operation". Path
            // reservations kept that from minting duplicates, but it cost two
            // extra full passes and a failed-looking job. Give it room.
            JobType::PackageInstall { .. } => 1800,
            // Default for everything else
            _ => 300,
        }
    }
}
