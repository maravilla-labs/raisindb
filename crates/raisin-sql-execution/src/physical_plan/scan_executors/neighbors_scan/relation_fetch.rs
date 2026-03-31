// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Relation fetch helpers for the neighbors scan executor.
//!
//! Fetches outgoing and incoming relations from the relation index,
//! with fallback to the global relation index when local lookups are empty.

use raisin_error::Error;
use raisin_hlc::HLC;
use raisin_storage::{RelationRepository, Storage};

/// Fetch outgoing relations for a source node, with fallback to global index.
pub(super) async fn fetch_outgoing_relations<S: Storage>(
    storage: &std::sync::Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    effective_source_id: &str,
    source_node_id: &str,
    relation_type: &Option<String>,
    max_revision_opt: Option<&HLC>,
    rev_ref: &HLC,
) -> Result<Vec<raisin_models::nodes::RelationRef>, Error> {
    let fetch = |id: &str| {
        let storage = storage.clone();
        let tenant_id = tenant_id.to_string();
        let repo_id = repo_id.to_string();
        let branch = branch.to_string();
        let source_workspace = source_workspace.to_string();
        let max_rev = max_revision_opt.cloned();
        let id = id.to_string();
        async move {
            storage
                .relations()
                .get_outgoing_relations(
                    raisin_storage::StorageScope::new(
                        &tenant_id,
                        &repo_id,
                        &branch,
                        &source_workspace,
                    ),
                    &id,
                    max_rev.as_ref(),
                )
                .await
        }
    };

    let mut result = fetch(effective_source_id).await?;
    if result.is_empty() && source_node_id.contains('/') && effective_source_id != source_node_id {
        result = fetch(source_node_id).await?;
    }

    // Fallback: use global index
    if result.is_empty() {
        if let Ok(global_rels) = storage
            .relations()
            .scan_relations_global(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                relation_type.as_deref(),
                Some(rev_ref),
            )
            .await
        {
            for (_src_ws, src_id, tgt_ws, tgt_id, full_rel) in global_rels {
                let matches_src = src_id == effective_source_id || src_id == source_node_id;
                let type_ok = relation_type
                    .as_ref()
                    .is_none_or(|t| full_rel.relation_type == *t);
                if matches_src && type_ok {
                    result.push(raisin_models::nodes::RelationRef {
                        target: tgt_id,
                        workspace: tgt_ws,
                        target_node_type: full_rel.target_node_type.clone(),
                        relation_type: full_rel.relation_type.clone(),
                        weight: full_rel.weight,
                    });
                }
            }
        }
    }

    Ok(result)
}

/// Fetch incoming relations for a target node, with fallback to global index.
pub(super) async fn fetch_incoming_relations<S: Storage>(
    storage: &std::sync::Arc<S>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    effective_source_id: &str,
    source_node_id: &str,
    relation_type: &Option<String>,
    max_revision_opt: Option<&HLC>,
    rev_ref: &HLC,
) -> Result<Vec<(String, String, raisin_models::nodes::RelationRef)>, Error> {
    let fetch = |id: &str| {
        let storage = storage.clone();
        let tenant_id = tenant_id.to_string();
        let repo_id = repo_id.to_string();
        let branch = branch.to_string();
        let source_workspace = source_workspace.to_string();
        let max_rev = max_revision_opt.cloned();
        let id = id.to_string();
        async move {
            storage
                .relations()
                .get_incoming_relations(
                    raisin_storage::StorageScope::new(
                        &tenant_id,
                        &repo_id,
                        &branch,
                        &source_workspace,
                    ),
                    &id,
                    max_rev.as_ref(),
                )
                .await
        }
    };

    let mut result = fetch(effective_source_id).await?;
    if result.is_empty() && source_node_id.contains('/') && effective_source_id != source_node_id {
        result = fetch(source_node_id).await?;
    }

    // Fallback: use global index
    if result.is_empty() {
        if let Ok(global_rels) = storage
            .relations()
            .scan_relations_global(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                relation_type.as_deref(),
                Some(rev_ref),
            )
            .await
        {
            for (src_ws, src_id, tgt_ws, tgt_id, full_rel) in global_rels {
                let matches_tgt = tgt_id == effective_source_id || tgt_id == source_node_id;
                let type_ok = relation_type
                    .as_ref()
                    .is_none_or(|t| full_rel.relation_type == *t);
                if matches_tgt && type_ok {
                    result.push((
                        src_ws,
                        src_id.clone(),
                        raisin_models::nodes::RelationRef {
                            target: tgt_id,
                            workspace: tgt_ws.clone(),
                            target_node_type: full_rel.target_node_type.clone(),
                            relation_type: full_rel.relation_type.clone(),
                            weight: full_rel.weight,
                        },
                    ));
                }
            }
        }
    }

    Ok(result)
}
