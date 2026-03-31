// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! RELATE and UNRELATE execution for managing node relationships.
//!
//! RELATE creates a relationship between two nodes (possibly cross-workspace).
//! UNRELATE removes a relationship between two nodes.

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, RelationRepository, Storage, StorageScope};

/// Execute a RELATE statement - create a relationship between nodes.
pub async fn execute_relate<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    source: &'a raisin_sql::analyzer::AnalyzedRelateEndpoint,
    target: &'a raisin_sql::analyzer::AnalyzedRelateEndpoint,
    relation_type: &'a str,
    weight: Option<f64>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    use raisin_models::nodes::RelationRef;
    use raisin_sql::ast::relate::RelateNodeReference;

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    tracing::debug!(
        "RELATE: FROM {}:{} TO {}:{} TYPE '{}' weight={:?}",
        source.workspace,
        source.node_ref,
        target.workspace,
        target.node_ref,
        relation_type,
        weight
    );

    // Step 1: Resolve source node
    let source_node = resolve_relate_node(&source.node_ref, &source.workspace, branch, ctx).await?;

    // Check RELATE permission on source node
    check_relate_permission(&source_node, ctx, &source.workspace, branch)?;

    let (source_id, source_node_type) = (source_node.id, source_node.node_type);

    // Step 2: Resolve target node
    let target_node = resolve_relate_node(&target.node_ref, &target.workspace, branch, ctx).await?;

    // Check READ permission on target node (to verify it can be referenced)
    check_read_permission(&target_node, ctx, &target.workspace, branch)?;

    let (target_id, target_node_type) = (target_node.id, target_node.node_type);

    // Step 3: Create RelationRef
    let relation = RelationRef::new(
        target_id.clone(),
        target.workspace.clone(),
        target_node_type,
        relation_type.to_string(),
        weight.map(|w| w as f32),
    );

    // Step 4: Add the relation
    ctx.storage
        .relations()
        .add_relation(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, &source.workspace),
            &source_id,
            &source_node_type,
            relation,
        )
        .await?;

    tracing::info!(
        "RELATE: Created relationship from {}:{} to {}:{} TYPE '{}'",
        source.workspace,
        source_id,
        target.workspace,
        target_id,
        relation_type
    );

    let mut result_row = Row::new();
    result_row.insert("affected_rows".to_string(), PropertyValue::Integer(1));

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Execute an UNRELATE statement - remove a relationship between nodes.
pub async fn execute_unrelate<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    source: &'a raisin_sql::analyzer::AnalyzedRelateEndpoint,
    target: &'a raisin_sql::analyzer::AnalyzedRelateEndpoint,
    relation_type: &'a Option<String>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    use raisin_sql::ast::relate::RelateNodeReference;

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    tracing::debug!(
        "UNRELATE: FROM {}:{} TO {}:{} TYPE={:?}",
        source.workspace,
        source.node_ref,
        target.workspace,
        target.node_ref,
        relation_type
    );

    // Step 1: Resolve source node
    let source_node = resolve_relate_node(&source.node_ref, &source.workspace, branch, ctx).await?;

    // Check UNRELATE permission on source node
    check_unrelate_permission(&source_node, ctx, &source.workspace, branch)?;

    let source_id = source_node.id;

    // Step 2: Resolve target node to get ID
    let target_id =
        resolve_relate_node_id(&target.node_ref, &target.workspace, branch, ctx).await?;

    // Step 3: Remove the relation
    if relation_type.is_some() {
        tracing::warn!(
            "UNRELATE with TYPE filter is not yet fully supported - removing all relations between nodes"
        );
    }

    let removed = ctx
        .storage
        .relations()
        .remove_relation(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, &source.workspace),
            &source_id,
            &target.workspace,
            &target_id,
        )
        .await?;

    let affected_rows = if removed { 1 } else { 0 };

    tracing::info!(
        "UNRELATE: Removed {} relationship(s) from {}:{} to {}:{}",
        affected_rows,
        source.workspace,
        source_id,
        target.workspace,
        target_id
    );

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(affected_rows),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Resolve a RelateNodeReference to a full Node.
async fn resolve_relate_node<S: Storage + 'static>(
    node_ref: &raisin_sql::ast::relate::RelateNodeReference,
    workspace: &str,
    branch: &str,
    ctx: &ExecutionContext<S>,
) -> Result<raisin_models::nodes::Node, Error> {
    use raisin_sql::ast::relate::RelateNodeReference;

    match node_ref {
        RelateNodeReference::Path(path) => ctx
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                path,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node at path '{}' not found", path))),
        RelateNodeReference::Id(id) => ctx
            .storage
            .nodes()
            .get(
                StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                id,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node with id '{}' not found", id))),
    }
}

/// Resolve a RelateNodeReference to just the node ID.
async fn resolve_relate_node_id<S: Storage + 'static>(
    node_ref: &raisin_sql::ast::relate::RelateNodeReference,
    workspace: &str,
    branch: &str,
    ctx: &ExecutionContext<S>,
) -> Result<String, Error> {
    use raisin_sql::ast::relate::RelateNodeReference;

    match node_ref {
        RelateNodeReference::Path(path) => {
            let node = ctx
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                    path,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("Target node at path '{}' not found", path))
                })?;
            Ok(node.id)
        }
        RelateNodeReference::Id(id) => {
            // Verify node exists
            let _node = ctx
                .storage
                .nodes()
                .get(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace),
                    id,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("Target node with id '{}' not found", id))
                })?;
            Ok(id.clone())
        }
    }
}

/// Check RELATE permission on a node via RLS.
fn check_relate_permission<S: Storage>(
    node: &raisin_models::nodes::Node,
    ctx: &ExecutionContext<S>,
    workspace: &str,
    branch: &str,
) -> Result<(), Error> {
    if let Some(ref auth) = ctx.auth_context {
        use raisin_core::services::rls_filter;
        use raisin_models::permissions::{Operation, PermissionScope};
        let scope = PermissionScope::new(workspace, branch);
        if !rls_filter::can_perform(node, Operation::Relate, auth, &scope) {
            return Err(Error::PermissionDenied(format!(
                "Cannot add relation from node in workspace '{}'",
                workspace
            )));
        }
    }
    Ok(())
}

/// Check READ permission on a node via RLS.
fn check_read_permission<S: Storage>(
    node: &raisin_models::nodes::Node,
    ctx: &ExecutionContext<S>,
    workspace: &str,
    branch: &str,
) -> Result<(), Error> {
    if let Some(ref auth) = ctx.auth_context {
        use raisin_core::services::rls_filter;
        use raisin_models::permissions::{Operation, PermissionScope};
        let scope = PermissionScope::new(workspace, branch);
        if !rls_filter::can_perform(node, Operation::Read, auth, &scope) {
            return Err(Error::PermissionDenied(format!(
                "Cannot relate to node in workspace '{}'",
                workspace
            )));
        }
    }
    Ok(())
}

/// Check UNRELATE permission on a node via RLS.
fn check_unrelate_permission<S: Storage>(
    node: &raisin_models::nodes::Node,
    ctx: &ExecutionContext<S>,
    workspace: &str,
    branch: &str,
) -> Result<(), Error> {
    if let Some(ref auth) = ctx.auth_context {
        use raisin_core::services::rls_filter;
        use raisin_models::permissions::{Operation, PermissionScope};
        let scope = PermissionScope::new(workspace, branch);
        if !rls_filter::can_perform(node, Operation::Unrelate, auth, &scope) {
            return Err(Error::PermissionDenied(format!(
                "Cannot remove relation from node in workspace '{}'",
                workspace
            )));
        }
    }
    Ok(())
}
