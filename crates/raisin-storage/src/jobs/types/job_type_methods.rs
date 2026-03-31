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

//! JobType methods for dedup_key and default_priority

use super::category::JobCategory;
use super::job_type::JobType;
use super::priority::JobPriority;

impl JobType {
    /// Returns a unique deduplication key for this job type
    pub fn dedup_key(&self) -> String {
        match self {
            Self::IntegrityScan => "integrity_scan".to_string(),
            Self::IndexRebuild => "index_rebuild".to_string(),
            Self::IndexVerify => "index_verify".to_string(),
            Self::Compaction => "compaction".to_string(),
            Self::Backup => "backup".to_string(),
            Self::Restore => "restore".to_string(),
            Self::OrphanCleanup => "orphan_cleanup".to_string(),
            Self::Repair => "repair".to_string(),
            Self::FulltextVerify => "fulltext_verify".to_string(),
            Self::FulltextRebuild => "fulltext_rebuild".to_string(),
            Self::FulltextOptimize => "fulltext_optimize".to_string(),
            Self::FulltextPurge => "fulltext_purge".to_string(),
            Self::VectorVerify => "vector_verify".to_string(),
            Self::VectorRebuild => "vector_rebuild".to_string(),
            Self::VectorOptimize => "vector_optimize".to_string(),
            Self::VectorRestore => "vector_restore".to_string(),
            Self::TreeSnapshot { revision } => format!("tree_snapshot:{}", revision.timestamp_ms),
            Self::FulltextIndex { node_id, operation } => {
                format!("fulltext_index:{}:{:?}", node_id, operation)
            }
            Self::EmbeddingGenerate { node_id } => format!("embedding_gen:{}", node_id),
            Self::EmbeddingDelete { node_id } => format!("embedding_del:{}", node_id),
            Self::NodeDeleteCleanup { node_id, workspace } => {
                format!("node_delete_cleanup:{}:{}", workspace, node_id)
            }
            Self::FulltextBranchCopy { source_branch } => {
                format!("fulltext_branch_copy:{}", source_branch)
            }
            Self::EmbeddingBranchCopy { source_branch } => {
                format!("embedding_branch_copy:{}", source_branch)
            }
            Self::FulltextBatchIndex { operation_count } => {
                format!("fulltext_batch:{}", operation_count)
            }
            Self::TriggerEvaluation {
                event_type,
                node_id,
                node_type,
            } => format!("trigger_eval:{}:{}:{}", event_type, node_id, node_type),
            Self::FunctionExecution {
                function_path,
                trigger_name,
                execution_id,
            } => format!(
                "func_exec:{}:{}:{}",
                function_path,
                trigger_name.as_deref().unwrap_or(""),
                execution_id
            ),
            Self::ScheduledTriggerCheck { tenant_id, repo_id } => format!(
                "sched_trigger:{}:{}",
                tenant_id.as_deref().unwrap_or("*"),
                repo_id.as_deref().unwrap_or("*")
            ),
            Self::AIToolCallExecution {
                tool_call_path,
                tool_call_workspace,
            } => format!("ai_tool_call:{}:{}", tool_call_workspace, tool_call_path),
            Self::AIToolResultAggregation {
                single_result_path,
                workspace,
            } => format!("ai_tool_agg:{}:{}", workspace, single_result_path),
            Self::AICall {
                instance_id,
                step_id,
                agent_ref,
                iteration,
            } => format!(
                "ai_call:{}:{}:{}:{}",
                instance_id, step_id, agent_ref, iteration
            ),
            Self::FlowExecution {
                flow_execution_id,
                trigger_path,
                current_step_index,
                ..
            } => format!(
                "flow_exec:{}:{}:{}",
                trigger_path, flow_execution_id, current_step_index
            ),
            Self::FlowInstanceExecution {
                instance_id,
                execution_type,
                resume_reason,
            } => format!(
                "flow_inst:{}:{}:{}",
                instance_id,
                execution_type,
                resume_reason.as_deref().unwrap_or("")
            ),
            Self::PropertyIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
            } => format!(
                "prop_idx:{}:{}:{}:{}",
                tenant_id, repo_id, branch, workspace
            ),
            Self::CompoundIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_type_name,
                index_name,
            } => format!(
                "compound_idx:{}:{}:{}:{}:{}:{}",
                tenant_id, repo_id, branch, workspace, node_type_name, index_name
            ),
            Self::RelationConsistencyCheck { repair } => format!("relation_check:{}", repair),
            Self::ReplicationGC { tenant_id, repo_id } => {
                format!("repl_gc:{}:{}", tenant_id, repo_id)
            }
            Self::ReplicationSync {
                tenant_id,
                repo_id,
                peer_id,
            } => format!(
                "repl_sync:{}:{}:{}",
                tenant_id,
                repo_id,
                peer_id.as_deref().unwrap_or("*")
            ),
            Self::OpLogCompaction { tenant_id, repo_id } => {
                format!("oplog_compact:{}:{}", tenant_id, repo_id)
            }
            Self::BulkSql { sql, actor } => {
                let sp = if sql.len() > 32 { &sql[..32] } else { sql };
                format!("bulk_sql:{}:{}", actor, sp)
            }
            Self::RevisionHistoryCopy {
                source_branch,
                target_branch,
                up_to_revision,
            } => format!(
                "rev_hist_copy:{}:{}:{}",
                source_branch, target_branch, up_to_revision.timestamp_ms
            ),
            Self::CopyTree {
                source_id,
                target_parent_id,
                new_name,
                recursive,
            } => format!(
                "copy_tree:{}:{}:{}:{}",
                source_id,
                target_parent_id,
                new_name.as_deref().unwrap_or(""),
                recursive
            ),
            Self::RestoreTree {
                node_id,
                node_path,
                revision_hlc,
                recursive,
                translations,
            } => format!(
                "restore_tree:{}:{}:{}:{}:{}",
                node_id,
                node_path,
                revision_hlc,
                recursive,
                translations
                    .as_ref()
                    .map(|t| t.join(","))
                    .unwrap_or_default()
            ),
            Self::PackageInstall {
                package_name,
                package_version,
                package_node_id,
            } => format!(
                "pkg_install:{}:{}:{}",
                package_name, package_version, package_node_id
            ),
            Self::PackageProcess { package_node_id } => format!("pkg_process:{}", package_node_id),
            Self::PackageExport {
                package_name,
                package_node_id,
                export_mode,
                ..
            } => format!(
                "pkg_export:{}:{}:{}",
                package_name, package_node_id, export_mode
            ),
            Self::PackageSyncStatus {
                package_node_id,
                compute_hashes,
            } => format!("pkg_sync_status:{}:{}", package_node_id, compute_hashes),
            Self::PackageSyncPush {
                package_node_id,
                paths_to_sync,
            } => format!("pkg_sync_push:{}:{}", package_node_id, paths_to_sync.len()),
            Self::PackageSyncPull {
                package_node_id,
                paths_to_pull,
                conflict_resolution,
            } => format!(
                "pkg_sync_pull:{}:{}:{}",
                package_node_id,
                paths_to_pull.len(),
                conflict_resolution
            ),
            Self::PackageCreateFromSelection {
                package_name,
                package_version,
                include_node_types,
            } => format!(
                "pkg_create_from_selection:{}:{}:{}",
                package_name, package_version, include_node_types
            ),
            Self::AuthMagicLinkSend {
                identity_id,
                email,
                token_id,
            } => format!("auth_magic_link:{}:{}:{}", identity_id, email, token_id),
            Self::AuthSessionCleanup {
                tenant_id,
                batch_size,
            } => format!(
                "auth_session_cleanup:{}:{}",
                tenant_id.as_deref().unwrap_or("*"),
                batch_size
            ),
            Self::AuthTokenCleanup {
                tenant_id,
                token_types,
            } => {
                let ts = if token_types.is_empty() {
                    "*".to_string()
                } else {
                    token_types.join(",")
                };
                format!(
                    "auth_token_cleanup:{}:{}",
                    tenant_id.as_deref().unwrap_or("*"),
                    ts
                )
            }
            Self::AuthAccessNotification {
                identity_id,
                repo_id,
                notification_type,
            } => format!(
                "auth_access_notif:{}:{}:{}",
                identity_id, repo_id, notification_type
            ),
            Self::AuthCreateUserNode {
                identity_id,
                repo_id,
                email,
                ..
            } => format!(
                "auth_create_user_node:{}:{}:{}",
                identity_id, repo_id, email
            ),
            Self::ResumableUploadComplete {
                upload_id,
                commit_message,
                commit_actor,
            } => format!(
                "resumable_upload_complete:{}:{}:{}",
                upload_id,
                commit_message.as_deref().unwrap_or(""),
                commit_actor.as_deref().unwrap_or("")
            ),
            Self::UploadSessionCleanup { upload_id } => {
                format!("upload_session_cleanup:{}", upload_id)
            }
            Self::HuggingFaceModelDownload { model_id } => {
                format!("huggingface_model_download:{}", model_id)
            }
            Self::HuggingFaceModelDelete { model_id } => {
                format!("huggingface_model_delete:{}", model_id)
            }
            Self::AssetProcessing { node_id, options } => format!(
                "asset_processing:{}:hash={}:pdf={}:img={}:cap={}",
                node_id,
                options.content_hash.as_deref().unwrap_or("none"),
                options.extract_pdf_text,
                options.generate_image_embedding,
                options.generate_image_caption
            ),
            Self::Custom(name) => format!("custom:{}", name),
        }
    }

    /// Returns the job category for pool isolation
    pub fn category(&self) -> JobCategory {
        match self {
            // Realtime: user-facing operations — triggers, functions, AI, flows
            Self::TriggerEvaluation { .. }
            | Self::FunctionExecution { .. }
            | Self::AIToolCallExecution { .. }
            | Self::AIToolResultAggregation { .. }
            | Self::AICall { .. }
            | Self::FlowExecution { .. }
            | Self::FlowInstanceExecution { .. } => JobCategory::Realtime,

            // Background: indexing, embedding, replication, maintenance
            Self::FulltextIndex { .. }
            | Self::FulltextBatchIndex { .. }
            | Self::FulltextBranchCopy { .. }
            | Self::FulltextVerify
            | Self::FulltextRebuild
            | Self::FulltextOptimize
            | Self::FulltextPurge
            | Self::EmbeddingGenerate { .. }
            | Self::EmbeddingDelete { .. }
            | Self::EmbeddingBranchCopy { .. }
            | Self::VectorVerify
            | Self::VectorRebuild
            | Self::VectorOptimize
            | Self::VectorRestore
            | Self::PropertyIndexBuild { .. }
            | Self::CompoundIndexBuild { .. }
            | Self::IntegrityScan
            | Self::IndexRebuild
            | Self::IndexVerify
            | Self::Compaction
            | Self::Backup
            | Self::Restore
            | Self::TreeSnapshot { .. }
            | Self::BulkSql { .. }
            | Self::CopyTree { .. }
            | Self::RestoreTree { .. }
            | Self::RevisionHistoryCopy { .. }
            | Self::RelationConsistencyCheck { .. }
            | Self::ReplicationGC { .. }
            | Self::ReplicationSync { .. }
            | Self::OpLogCompaction { .. }
            | Self::HuggingFaceModelDownload { .. }
            | Self::HuggingFaceModelDelete { .. }
            | Self::AssetProcessing { .. } => JobCategory::Background,

            // System: auth, packages, cleanup, scheduled checks
            Self::AuthMagicLinkSend { .. }
            | Self::AuthSessionCleanup { .. }
            | Self::AuthTokenCleanup { .. }
            | Self::AuthAccessNotification { .. }
            | Self::AuthCreateUserNode { .. }
            | Self::PackageInstall { .. }
            | Self::PackageProcess { .. }
            | Self::PackageExport { .. }
            | Self::PackageSyncStatus { .. }
            | Self::PackageSyncPush { .. }
            | Self::PackageSyncPull { .. }
            | Self::PackageCreateFromSelection { .. }
            | Self::NodeDeleteCleanup { .. }
            | Self::UploadSessionCleanup { .. }
            | Self::ResumableUploadComplete { .. }
            | Self::ScheduledTriggerCheck { .. }
            | Self::OrphanCleanup
            | Self::Repair
            | Self::Custom(_) => JobCategory::System,
        }
    }

    /// Returns the default priority for this job type
    pub fn default_priority(&self) -> JobPriority {
        match self {
            Self::FunctionExecution { .. }
            | Self::TriggerEvaluation { .. }
            | Self::FlowExecution { .. }
            | Self::FlowInstanceExecution { .. }
            | Self::AICall { .. }
            | Self::AIToolCallExecution { .. }
            | Self::AIToolResultAggregation { .. }
            | Self::ResumableUploadComplete { .. } => JobPriority::High,
            Self::FulltextIndex { .. }
            | Self::FulltextBatchIndex { .. }
            | Self::FulltextBranchCopy { .. }
            | Self::FulltextVerify
            | Self::FulltextRebuild
            | Self::FulltextOptimize
            | Self::FulltextPurge
            | Self::EmbeddingGenerate { .. }
            | Self::EmbeddingDelete { .. }
            | Self::EmbeddingBranchCopy { .. }
            | Self::VectorVerify
            | Self::VectorRebuild
            | Self::VectorOptimize
            | Self::VectorRestore
            | Self::PropertyIndexBuild { .. }
            | Self::CompoundIndexBuild { .. }
            | Self::IntegrityScan
            | Self::IndexRebuild
            | Self::IndexVerify
            | Self::Compaction
            | Self::OrphanCleanup
            | Self::NodeDeleteCleanup { .. }
            | Self::RelationConsistencyCheck { .. }
            | Self::ReplicationGC { .. }
            | Self::OpLogCompaction { .. }
            | Self::TreeSnapshot { .. }
            | Self::AuthSessionCleanup { .. }
            | Self::AuthTokenCleanup { .. }
            | Self::UploadSessionCleanup { .. }
            | Self::AssetProcessing { .. } => JobPriority::Low,
            _ => JobPriority::Normal,
        }
    }
}
