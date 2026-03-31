// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Neighbors scan executor.
//!
//! Uses the RELATION_INDEX column family to find connected nodes via graph edges.
//! Supports outgoing (OUT), incoming (IN), and bidirectional (BOTH) traversal.
//!
//! # Module Structure
//!
//! - `relation_fetch` - Helpers for fetching outgoing/incoming relations with global fallback

mod relation_fetch;

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT, TIME_CHECK_INTERVAL};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::time::Instant;

/// Execute a NeighborsScan operator.
///
/// Uses the RELATION_INDEX column family to find connected nodes via graph edges.
/// Supports outgoing, incoming, and bidirectional traversal with optional
/// relation_type filtering.
pub async fn execute_neighbors_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (
        tenant_id,
        repo_id,
        branch,
        alias,
        source_workspace,
        source_node_id,
        direction,
        relation_type,
        projection,
        limit,
    ) = match plan {
        PhysicalPlan::NeighborsScan {
            tenant_id,
            repo_id,
            branch,
            alias,
            source_workspace,
            source_node_id,
            direction,
            relation_type,
            projection,
            limit,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            alias.clone(),
            source_workspace.clone(),
            source_node_id.clone(),
            direction.clone(),
            relation_type.clone(),
            projection.clone(),
            *limit,
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for neighbors scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_revision_opt = ctx.max_revision;
    let ctx_clone = ctx.clone();

    Ok(Box::pin(try_stream! {
        let direction_upper = direction.to_uppercase();
        let locales_to_use = get_locales_to_use(&ctx_clone);
        let fallback_rev = max_revision_opt
            .unwrap_or_else(|| HLC::new(u64::MAX, u64::MAX));
        let rev_ref = max_revision_opt.as_ref().unwrap_or(&fallback_rev);
        let mut emitted = 0usize;
        let mut safety_scanned = 0usize;
        let start_time = Instant::now();

        let qualifier = alias.clone().unwrap_or_else(|| source_workspace.clone());

        // Resolve path to node id if caller passed a path
        let mut effective_source_id = source_node_id.clone();
        if source_node_id.contains('/') {
            if let Some(node) = storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &source_workspace),
                    &source_node_id,
                    max_revision_opt.as_ref(),
                )
                .await?
            {
                effective_source_id = node.id.clone();
            }
        }

        // --- Outgoing relations ---
        if direction_upper == "OUT" || direction_upper == "BOTH" {
            let outgoing = relation_fetch::fetch_outgoing_relations(
                &storage, &tenant_id, &repo_id, &branch, &source_workspace,
                &effective_source_id, &source_node_id, &relation_type,
                max_revision_opt.as_ref(), rev_ref,
            ).await?;

            for relation in outgoing {
                if let Some(lim) = limit {
                    if emitted >= lim {
                        break;
                    }
                }

                safety_scanned += 1;
                if safety_scanned > SCAN_COUNT_CEILING {
                    tracing::warn!("NeighborsScan count limit reached: {} relations checked", safety_scanned);
                    break;
                }
                if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    tracing::warn!("NeighborsScan time limit reached: {:?} elapsed", start_time.elapsed());
                    break;
                }

                if let Some(ref rel_type) = relation_type {
                    if &relation.relation_type != rel_type {
                        continue;
                    }
                }

                let target_node = storage
                    .nodes()
                    .get_by_path(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &relation.workspace),
                        &relation.target, max_revision_opt.as_ref(),
                    )
                    .await?;

                if let Some(node) = target_node {
                    if node.path == "/" { continue; }

                    let node = if let Some(ref auth) = ctx_clone.auth_context {
                        let scope = PermissionScope::new(&relation.workspace, &branch);
                        match rls_filter::filter_node(node, auth, &scope) {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        node
                    };

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let mut row = node_to_row(&translated_node, &qualifier, &relation.workspace, &projection, &ctx_clone, locale).await?;
                        insert_neighbor_columns(&mut row, &translated_node, &qualifier, &relation.relation_type, relation.weight);
                        yield row;
                        emitted += 1;
                        if let Some(lim) = limit {
                            if emitted >= lim { break; }
                        }
                    }
                }
            }
        }

        // --- Incoming relations ---
        if direction_upper == "IN" || direction_upper == "BOTH" {
            let incoming = relation_fetch::fetch_incoming_relations(
                &storage, &tenant_id, &repo_id, &branch, &source_workspace,
                &effective_source_id, &source_node_id, &relation_type,
                max_revision_opt.as_ref(), rev_ref,
            ).await?;

            for (src_workspace, src_id, relation) in incoming {
                if let Some(lim) = limit {
                    if emitted >= lim { break; }
                }

                safety_scanned += 1;
                if safety_scanned > SCAN_COUNT_CEILING {
                    tracing::warn!("NeighborsScan count limit reached: {} relations checked", safety_scanned);
                    break;
                }
                if safety_scanned % TIME_CHECK_INTERVAL == 0 && start_time.elapsed() > SCAN_TIME_LIMIT {
                    tracing::warn!("NeighborsScan time limit reached: {:?} elapsed", start_time.elapsed());
                    break;
                }

                if let Some(ref rel_type) = relation_type {
                    if &relation.relation_type != rel_type { continue; }
                }

                let source_node = storage
                    .nodes()
                    .get_by_path(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &src_workspace),
                        &src_id, max_revision_opt.as_ref(),
                    )
                    .await?;

                if let Some(node) = source_node {
                    if node.path == "/" { continue; }

                    let node = if let Some(ref auth) = ctx_clone.auth_context {
                        let scope = PermissionScope::new(&src_workspace, &branch);
                        match rls_filter::filter_node(node, auth, &scope) {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        node
                    };

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                            Some(n) => n,
                            None => continue,
                        };

                        let mut row = node_to_row(&translated_node, &qualifier, &src_workspace, &projection, &ctx_clone, locale).await?;
                        insert_neighbor_columns(&mut row, &translated_node, &qualifier, &relation.relation_type, relation.weight);
                        yield row;
                        emitted += 1;
                        if let Some(lim) = limit {
                            if emitted >= lim { break; }
                        }
                    }
                }
            }
        }
    }))
}

/// Insert unqualified core columns and relation metadata into a neighbor row.
fn insert_neighbor_columns(
    row: &mut Row,
    node: &raisin_models::nodes::Node,
    qualifier: &str,
    relation_type: &str,
    weight: Option<f32>,
) {
    row.insert("id".to_string(), PropertyValue::String(node.id.clone()));
    row.insert("path".to_string(), PropertyValue::String(node.path.clone()));
    row.insert(
        "node_type".to_string(),
        PropertyValue::String(node.node_type.clone()),
    );
    row.insert(
        "__node_type".to_string(),
        PropertyValue::String(node.node_type.clone()),
    );
    row.insert("name".to_string(), PropertyValue::String(node.name.clone()));
    row.insert(
        "properties".to_string(),
        PropertyValue::Object(node.properties.clone()),
    );
    if let Some(created_at) = node.created_at {
        row.insert(
            "created_at".to_string(),
            PropertyValue::Date(created_at.into()),
        );
    }
    if let Some(updated_at) = node.updated_at {
        row.insert(
            "updated_at".to_string(),
            PropertyValue::Date(updated_at.into()),
        );
    }

    // Relation metadata
    row.insert(
        format!("{}.relation_type", qualifier),
        PropertyValue::String(relation_type.to_string()),
    );
    row.insert(
        "relation_type".to_string(),
        PropertyValue::String(relation_type.to_string()),
    );
    if let Some(w) = weight {
        row.insert("weight".to_string(), PropertyValue::Float(w as f64));
        row.insert(
            format!("{}.weight", qualifier),
            PropertyValue::Float(w as f64),
        );
    }
}
