// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Initial structure helpers for creating child nodes from NodeType definitions.
//!
//! When a new node is inserted via SQL, if its NodeType defines
//! `initial_structure.children`, those children are automatically created.

use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{transactional::TransactionalContext, NodeTypeRepository, Storage};
use std::collections::HashMap;

/// Creates initial_structure children for a node based on its NodeType definition.
///
/// This function:
/// 1. Fetches the NodeType for the given node
/// 2. If initial_structure.children is defined, creates each child node
/// 3. Recursively creates initial_structure for child nodes
///
/// This ensures consistency with NodeService::add_node() which also creates initial_structure.
pub(super) async fn create_initial_structure_children<S: Storage>(
    txn_ctx: &dyn TransactionalContext,
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_node: &raisin_models::nodes::Node,
) -> Result<(), Error> {
    // Fetch the NodeType for this node
    let node_type_def = storage
        .node_types()
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &parent_node.node_type,
            None,
        )
        .await?;

    let node_type_def = match node_type_def {
        Some(nt) => nt,
        None => {
            tracing::warn!(
                "NodeType '{}' not found when creating initial_structure for node at {}",
                parent_node.node_type,
                parent_node.path
            );
            return Ok(());
        }
    };

    let initial_structure = match &node_type_def.initial_structure {
        Some(is) => is,
        None => return Ok(()),
    };

    let children = match &initial_structure.children {
        Some(c) => c,
        None => return Ok(()),
    };

    for child_def in children {
        let child_node = build_initial_child_node(child_def, parent_node, workspace)?;

        txn_ctx.add_node(workspace, &child_node).await?;

        // Recursively create initial_structure for the child
        Box::pin(create_initial_structure_children(
            txn_ctx,
            storage,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &child_node,
        ))
        .await?;
    }

    Ok(())
}

/// Build a child node from an InitialChild definition.
fn build_initial_child_node(
    child_def: &raisin_models::nodes::types::initial_structure::InitialChild,
    parent_node: &raisin_models::nodes::Node,
    workspace: &str,
) -> Result<raisin_models::nodes::Node, Error> {
    use raisin_models::nodes::Node;

    let properties: HashMap<String, PropertyValue> = child_def
        .properties
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| serde_json::from_value(v).ok().map(|pv| (k, pv)))
        .collect();

    let translations = child_def.translations.as_ref().map(|trans| {
        trans
            .iter()
            .filter_map(|(k, v)| {
                serde_json::from_value(v.clone())
                    .ok()
                    .map(|pv| (k.clone(), pv))
            })
            .collect()
    });

    let child_path = format!("{}/{}", parent_node.path, child_def.name);
    let now = chrono::Utc::now();

    let child_node = Node {
        id: nanoid::nanoid!(),
        name: child_def.name.clone(),
        path: child_path,
        node_type: child_def.node_type.clone(),
        archetype: child_def.archetype.clone(),
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some(parent_node.name.clone()),
        version: 1,
        created_at: Some(now),
        updated_at: Some(now),
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations,
        tenant_id: parent_node.tenant_id.clone(),
        workspace: Some(workspace.to_string()),
        owner_id: parent_node.owner_id.clone(),
        relations: vec![],
    };

    Ok(child_node)
}
