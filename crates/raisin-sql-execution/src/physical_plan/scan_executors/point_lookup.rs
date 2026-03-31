// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Point lookup scan executors.
//!
//! O(1) lookups for exact path matches and direct node ID access.
//! These are the fastest possible access methods for known identifiers.

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a PathIndexScan operator.
///
/// Uses the path_index column family for exact path lookups.
/// Optimal for queries like: `WHERE path = '/exact/path'`
///
/// # Performance
/// - O(1) lookup time using path_index CF
/// - Much faster than PrefixScan or TableScan for exact matches
pub async fn execute_path_index_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (tenant_id, repo_id, branch, workspace, table, alias, path, projection) = match plan {
        PhysicalPlan::PathIndexScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            path,
            projection,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            path.clone(),
            projection.clone(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for path index scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let ctx_clone = ctx.clone();

    tracing::info!(
        "   PathIndexScan: path='{}', workspace='{}', branch='{}', max_revision={:?}",
        path,
        workspace,
        branch,
        max_revision
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        tracing::debug!("   Looking up node by exact path '{}'...", path);

        let node_opt = storage
            .nodes()
            .get_by_path(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                &path,
                max_revision.as_ref(),
            )
            .await?;

        if let Some(node) = node_opt {
            if node.path != "/" {
                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match rls_filter::filter_node(node, auth, &scope) {
                        Some(n) => n,
                        None => {
                            tracing::debug!("   PathIndexScan: RLS filtered out node at path '{}'", path);
                            return;
                        }
                    }
                } else {
                    node
                };

                tracing::info!("   PathIndexScan found node: id={}", node.id);

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                    yield row;
                }
            } else {
                tracing::debug!("   PathIndexScan: skipping root node");
            }
        } else {
            tracing::debug!("   PathIndexScan: no node found at path '{}'", path);
        }
    }))
}

/// Execute a NodeIdScan operator.
///
/// Uses the NODES column family for direct node lookups by ID.
/// Optimal for queries like: `WHERE id = 'uuid'`
///
/// # Performance
/// - O(1) lookup time using NODES CF with direct key access
/// - Fastest possible access method for known node IDs
pub async fn execute_node_id_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (tenant_id, repo_id, branch, workspace, table, alias, node_id, projection) = match plan {
        PhysicalPlan::NodeIdScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            node_id,
            projection,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            node_id.clone(),
            projection.clone(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for node ID scan".to_string(),
            ))
        }
    };

    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let ctx_clone = ctx.clone();

    tracing::info!(
        "   NodeIdScan: id='{}', workspace='{}', branch='{}', max_revision={:?}",
        node_id,
        workspace,
        branch,
        max_revision
    );

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let locales_to_use = get_locales_to_use(&ctx_clone);

        tracing::debug!("   Looking up node by ID '{}'...", node_id);

        let node_opt = storage
            .nodes()
            .get(
                StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
                &node_id,
                max_revision.as_ref(),
            )
            .await?;

        if let Some(node) = node_opt {
            if node.path != "/" {
                let node = if let Some(ref auth) = ctx_clone.auth_context {
                    let scope = PermissionScope::new(&workspace, &branch);
                    match rls_filter::filter_node(node, auth, &scope) {
                        Some(n) => n,
                        None => {
                            tracing::debug!("   NodeIdScan: RLS filtered out node id '{}'", node_id);
                            return;
                        }
                    }
                } else {
                    node
                };

                tracing::info!("   NodeIdScan found node: path={}", node.path);

                for locale in &locales_to_use {
                    let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };

                    let row = node_to_row(&translated_node, &qualifier, &workspace, &projection, &ctx_clone, locale).await?;
                    yield row;
                }
            } else {
                tracing::debug!("   NodeIdScan: skipping root node");
            }
        } else {
            tracing::debug!("   NodeIdScan: no node found with id '{}'", node_id);
        }
    }))
}
