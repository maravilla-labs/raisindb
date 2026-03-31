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
    Custom(String),
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
            JobType::UploadSessionCleanup { .. } => 120,
            // Function/AI execution — can involve AI API calls (10-30s each)
            JobType::FunctionExecution { .. } => 300,
            JobType::AIToolCallExecution { .. } => 300,
            JobType::AICall { .. } => 300,
            JobType::FlowExecution { .. } => 300,
            JobType::FlowInstanceExecution { .. } => 300,
            // Long-running background/maintenance jobs
            JobType::IntegrityScan => 600,
            JobType::IndexRebuild => 600,
            JobType::IndexVerify => 600,
            JobType::FulltextRebuild => 600,
            JobType::VectorRebuild => 600,
            JobType::Backup => 600,
            JobType::Restore => 600,
            // Default for everything else
            _ => 300,
        }
    }
}
