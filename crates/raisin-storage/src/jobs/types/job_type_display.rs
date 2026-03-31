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

//! Display implementation for JobType

use std::fmt;

use super::job_type::JobType;

impl fmt::Display for JobType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntegrityScan => write!(f, "IntegrityScan"),
            Self::IndexRebuild => write!(f, "IndexRebuild"),
            Self::IndexVerify => write!(f, "IndexVerify"),
            Self::Compaction => write!(f, "Compaction"),
            Self::Backup => write!(f, "Backup"),
            Self::Restore => write!(f, "Restore"),
            Self::OrphanCleanup => write!(f, "OrphanCleanup"),
            Self::Repair => write!(f, "Repair"),
            Self::TreeSnapshot { revision } => write!(f, "TreeSnapshot({})", revision.timestamp_ms),
            Self::FulltextVerify => write!(f, "FulltextVerify"),
            Self::FulltextRebuild => write!(f, "FulltextRebuild"),
            Self::FulltextOptimize => write!(f, "FulltextOptimize"),
            Self::FulltextPurge => write!(f, "FulltextPurge"),
            Self::VectorVerify => write!(f, "VectorVerify"),
            Self::VectorRebuild => write!(f, "VectorRebuild"),
            Self::VectorOptimize => write!(f, "VectorOptimize"),
            Self::VectorRestore => write!(f, "VectorRestore"),
            Self::FulltextIndex { node_id, operation } => {
                write!(f, "FulltextIndex({}, {:?})", node_id, operation)
            }
            Self::FulltextBranchCopy { source_branch } => {
                write!(f, "FulltextBranchCopy({})", source_branch)
            }
            Self::FulltextBatchIndex { operation_count } => {
                write!(f, "FulltextBatchIndex(count={})", operation_count)
            }
            Self::EmbeddingGenerate { node_id } => write!(f, "EmbeddingGenerate({})", node_id),
            Self::EmbeddingDelete { node_id } => write!(f, "EmbeddingDelete({})", node_id),
            Self::EmbeddingBranchCopy { source_branch } => {
                write!(f, "EmbeddingBranchCopy({})", source_branch)
            }
            Self::HuggingFaceModelDownload { model_id } => {
                write!(f, "HuggingFaceModelDownload({})", model_id)
            }
            Self::HuggingFaceModelDelete { model_id } => {
                write!(f, "HuggingFaceModelDelete({})", model_id)
            }
            Self::AssetProcessing { node_id, options } => write!(
                f,
                "AssetProcessing({}, pdf={}, img_embed={}, caption={})",
                node_id,
                options.extract_pdf_text,
                options.generate_image_embedding,
                options.generate_image_caption
            ),
            Self::ReplicationGC { tenant_id, repo_id } => {
                write!(f, "ReplicationGC({}/{})", tenant_id, repo_id)
            }
            Self::ReplicationSync {
                tenant_id,
                repo_id,
                peer_id,
            } => {
                if let Some(peer) = peer_id {
                    write!(f, "ReplicationSync({}/{}/{})", tenant_id, repo_id, peer)
                } else {
                    write!(f, "ReplicationSync({}/{})", tenant_id, repo_id)
                }
            }
            Self::OpLogCompaction { tenant_id, repo_id } => {
                write!(f, "OpLogCompaction({}/{})", tenant_id, repo_id)
            }
            Self::PropertyIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
            } => write!(
                f,
                "PropertyIndexBuild({}/{}/{}/{})",
                tenant_id, repo_id, branch, workspace
            ),
            Self::CompoundIndexBuild {
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_type_name,
                index_name,
            } => write!(
                f,
                "CompoundIndexBuild({}/{}/{}/{}/{}/{})",
                tenant_id, repo_id, branch, workspace, node_type_name, index_name
            ),
            Self::BulkSql { sql, actor } => {
                let sql_preview = if sql.len() > 50 {
                    format!("{}...", &sql[..50])
                } else {
                    sql.clone()
                };
                write!(f, "BulkSql({}, {})", actor, sql_preview)
            }
            Self::RevisionHistoryCopy {
                source_branch,
                target_branch,
                up_to_revision,
            } => write!(
                f,
                "RevisionHistoryCopy({}/{}/{})",
                source_branch, target_branch, up_to_revision.timestamp_ms
            ),
            Self::CopyTree {
                source_id,
                target_parent_id,
                new_name,
                recursive,
            } => {
                let name_part = new_name
                    .as_ref()
                    .map(|n| format!("/{}", n))
                    .unwrap_or_default();
                let recursive_flag = if *recursive { "R" } else { "" };
                write!(
                    f,
                    "CopyTree({}/{}{}{})",
                    source_id, target_parent_id, name_part, recursive_flag
                )
            }
            Self::RestoreTree {
                node_id,
                node_path,
                revision_hlc,
                recursive,
                translations,
            } => {
                let recursive_flag = if *recursive { "R" } else { "" };
                let trans_part = translations
                    .as_ref()
                    .map(|t| format!("/T:{}", t.join(",")))
                    .unwrap_or_default();
                write!(
                    f,
                    "RestoreTree({}/{}/{}{}{})",
                    node_id, node_path, revision_hlc, recursive_flag, trans_part
                )
            }
            Self::NodeDeleteCleanup { node_id, workspace } => {
                write!(f, "NodeDeleteCleanup({}/{})", node_id, workspace)
            }
            Self::RelationConsistencyCheck { repair } => write!(
                f,
                "RelationConsistencyCheck(repair={})",
                if *repair { "true" } else { "false" }
            ),
            Self::FunctionExecution {
                function_path,
                trigger_name,
                execution_id,
            } => {
                if let Some(trigger) = trigger_name {
                    write!(
                        f,
                        "FunctionExecution({}/{}/{})",
                        function_path, trigger, execution_id
                    )
                } else {
                    write!(f, "FunctionExecution({}/{})", function_path, execution_id)
                }
            }
            Self::TriggerEvaluation {
                event_type,
                node_id,
                node_type,
            } => write!(
                f,
                "TriggerEvaluation({}/{}/{})",
                event_type, node_id, node_type
            ),
            Self::ScheduledTriggerCheck { tenant_id, repo_id } => match (tenant_id, repo_id) {
                (Some(t), Some(r)) => write!(f, "ScheduledTriggerCheck({}/{})", t, r),
                (Some(t), None) => write!(f, "ScheduledTriggerCheck({})", t),
                (None, Some(r)) => write!(f, "ScheduledTriggerCheck(*/{})", r),
                (None, None) => write!(f, "ScheduledTriggerCheck(*)"),
            },
            Self::FlowExecution {
                flow_execution_id,
                trigger_path,
                current_step_index,
                ..
            } => write!(
                f,
                "FlowExecution({}/{}/step:{})",
                trigger_path, flow_execution_id, current_step_index
            ),
            Self::FlowInstanceExecution {
                instance_id,
                execution_type,
                resume_reason,
            } => {
                if let Some(reason) = resume_reason {
                    write!(
                        f,
                        "FlowInstanceExecution({}/{}/{})",
                        instance_id, execution_type, reason
                    )
                } else {
                    write!(
                        f,
                        "FlowInstanceExecution({}/{})",
                        instance_id, execution_type
                    )
                }
            }
            Self::AICall {
                instance_id,
                step_id,
                agent_ref,
                iteration,
            } => write!(
                f,
                "AICall({}/{}/{}/{})",
                instance_id, step_id, agent_ref, iteration
            ),
            Self::PackageInstall {
                package_name,
                package_version,
                ..
            } => write!(f, "PackageInstall({}/{})", package_name, package_version),
            Self::PackageProcess { package_node_id } => {
                write!(f, "PackageProcess({})", package_node_id)
            }
            Self::PackageExport {
                package_name,
                export_mode,
                ..
            } => write!(f, "PackageExport({}/{})", package_name, export_mode),
            Self::PackageSyncStatus {
                package_node_id, ..
            } => write!(f, "PackageSyncStatus({})", package_node_id),
            Self::PackageSyncPush {
                package_node_id,
                paths_to_sync,
            } => write!(
                f,
                "PackageSyncPush({}/{})",
                package_node_id,
                paths_to_sync.len()
            ),
            Self::PackageSyncPull {
                package_node_id,
                paths_to_pull,
                conflict_resolution,
            } => write!(
                f,
                "PackageSyncPull({}/{}/{})",
                package_node_id,
                paths_to_pull.len(),
                conflict_resolution
            ),
            Self::PackageCreateFromSelection {
                package_name,
                package_version,
                include_node_types,
            } => write!(
                f,
                "PackageCreateFromSelection({}/{}/{})",
                package_name,
                package_version,
                if *include_node_types {
                    "types"
                } else {
                    "no-types"
                }
            ),
            Self::AIToolCallExecution {
                tool_call_path,
                tool_call_workspace,
            } => write!(
                f,
                "AIToolCallExecution({}/{})",
                tool_call_workspace, tool_call_path
            ),
            Self::AIToolResultAggregation {
                single_result_path,
                workspace,
            } => write!(
                f,
                "AIToolResultAggregation({}/{})",
                workspace, single_result_path
            ),
            Self::AuthMagicLinkSend {
                identity_id,
                email,
                token_id,
            } => write!(
                f,
                "AuthMagicLinkSend({}/{}/{})",
                identity_id, email, token_id
            ),
            Self::AuthSessionCleanup {
                tenant_id,
                batch_size,
            } => {
                if let Some(tenant) = tenant_id {
                    write!(f, "AuthSessionCleanup({}/{})", tenant, batch_size)
                } else {
                    write!(f, "AuthSessionCleanup(*/{})", batch_size)
                }
            }
            Self::AuthTokenCleanup {
                tenant_id,
                token_types,
            } => {
                let types_str = if token_types.is_empty() {
                    "*".to_string()
                } else {
                    token_types.join(",")
                };
                if let Some(tenant) = tenant_id {
                    write!(f, "AuthTokenCleanup({}/{})", tenant, types_str)
                } else {
                    write!(f, "AuthTokenCleanup(*/{})", types_str)
                }
            }
            Self::AuthAccessNotification {
                identity_id,
                repo_id,
                notification_type,
            } => write!(
                f,
                "AuthAccessNotification({}/{}/{})",
                identity_id, repo_id, notification_type
            ),
            Self::AuthCreateUserNode {
                identity_id,
                repo_id,
                email,
                ..
            } => write!(
                f,
                "AuthCreateUserNode({}/{}/{})",
                identity_id, repo_id, email
            ),
            Self::ResumableUploadComplete {
                upload_id,
                commit_message,
                commit_actor,
            } => {
                let msg_part = commit_message
                    .as_ref()
                    .map(|m| format!("/{}", m))
                    .unwrap_or_default();
                let actor_part = commit_actor
                    .as_ref()
                    .map(|a| format!("/{}", a))
                    .unwrap_or_default();
                write!(
                    f,
                    "ResumableUploadComplete({}{}{})",
                    upload_id, msg_part, actor_part
                )
            }
            Self::UploadSessionCleanup { upload_id } => {
                write!(f, "UploadSessionCleanup({})", upload_id)
            }
            Self::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}
