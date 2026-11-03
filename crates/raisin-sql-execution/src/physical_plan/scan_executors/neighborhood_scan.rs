// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Neighbourhood scan executor: the set form of `NEIGHBORS`.
//!
//! ```sql
//! SELECT id, depth, via_count
//!   FROM NEIGHBORS('people:/ann', 'OUT', depth => 2, types => 'CONNECTED', min_depth => 2)
//! ```
//!
//! returns every node within two hops of the start exactly once, with its
//! shortest `depth` and `via_count` (distinct nodes one level closer that reach
//! it: the mutual-connection count at depth 2). The search itself lives in
//! [`crate::physical_plan::graph_neighborhood`]; this file supplies the pieces
//! that need the engine: the start node, the row-level-security gate and the
//! row shape.
//!
//! # Row-level security
//!
//! The gate below loads a node and passes it through the caller's RLS; the
//! root node is never reported. It is asked about the START first: a start the
//! caller cannot read gives the same empty result as a start that does not
//! exist, and nothing is read from it. Reported nodes are gated as `NEIGHBORS`
//! does today. A node the caller cannot read is left out of the result but is
//! still walked through, which is the endpoints-only policy `GRAPH_TABLE`
//! documents in `pgq::rls`: the two surfaces must not disagree about who is two
//! hops away. `via_count` counts readable nodes only.
//!
//! # Limits
//!
//! The search runs under [`SCAN_TIME_LIMIT`] and a relation-read ceiling of
//! [`SCAN_COUNT_CEILING`] (or the caller's `max_nodes`, when that is larger),
//! like every other scan. `max_nodes` above [`DEFAULT_MAX_NODES`] is honoured
//! only for a system or system-admin caller; anyone else is held to the
//! default. Rows are produced chunk by chunk as the search hands nodes out.

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use super::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT};
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use crate::physical_plan::graph_access::{
    resolve_ids, resolve_paths, NodeKey, Side, StorageAccess,
};
use crate::physical_plan::graph_budget::GraphBudget;
use crate::physical_plan::graph_neighborhood::{
    NeighborhoodParams, NeighborhoodSearch, NodeGate, DEFAULT_MAX_NODES,
};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Lets the nodes the caller may see into the result, handing back the node.
struct RlsGate<'a, S: Storage> {
    ctx: &'a ExecutionContext<S>,
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    revision: Option<&'a HLC>,
}

impl<S: Storage + 'static> NodeGate for RlsGate<'_, S> {
    type Item = Node;

    async fn report(
        &self,
        workspace: &str,
        id: &str,
        _depth: u32,
    ) -> Result<Option<Node>, ExecutionError> {
        let node = self
            .ctx
            .storage
            .nodes()
            .get(
                StorageScope::new(self.tenant_id, self.repo_id, self.branch, workspace),
                id,
                self.revision,
            )
            .await?;
        let Some(node) = node else {
            return Ok(None);
        };
        if node.path == "/" {
            return Ok(None);
        }
        match &self.ctx.auth_context {
            Some(auth) => {
                let scope = PermissionScope::new(workspace, self.branch);
                Ok(super::helpers::rls_filter_node_graph(
                    &*self.ctx.storage,
                    node,
                    auth,
                    &scope,
                    self.tenant_id,
                    self.repo_id,
                    self.branch,
                    self.revision,
                )
                .await)
            }
            None => Ok(Some(node)),
        }
    }
}

/// Execute a `NeighborhoodScan`.
pub async fn execute_neighborhood_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let PhysicalPlan::NeighborhoodScan {
        tenant_id,
        repo_id,
        branch,
        alias,
        source_workspace,
        source_node_id,
        direction,
        relation_types,
        min_depth,
        max_depth,
        max_nodes,
    } = plan
    else {
        return Err(ExecutionError::Validation(
            "Invalid plan for neighborhood scan".to_string(),
        ));
    };

    let (tenant_id, repo_id, branch) = (tenant_id.clone(), repo_id.clone(), branch.clone());
    let alias = alias.clone();
    let source_workspace = source_workspace.clone();
    let source_node_id = source_node_id.clone();
    let side = match direction.as_str() {
        "OUT" => Side::Out,
        "IN" => Side::In,
        _ => Side::Both,
    };
    let max_nodes = effective_max_nodes(*max_nodes, ctx.auth_context.as_ref());
    let types = relation_types.clone();
    let (min_depth, max_depth) = (*min_depth, *max_depth);
    let ctx = ctx.clone();

    Ok(Box::pin(try_stream! {
        // The clock starts when the stream is first polled, as for every scan.
        let params = NeighborhoodParams {
            side,
            types,
            min_depth,
            max_depth,
            max_nodes,
            budget: GraphBudget::new(SCAN_TIME_LIMIT, SCAN_COUNT_CEILING.max(max_nodes)),
        };
        let revision = ctx.max_revision;
        let access = StorageAccess::new(&ctx.storage, &tenant_id, &repo_id, &branch, revision.as_ref())
            .with_workspace_hint(&source_workspace);

        // The start: a path is looked up in the given workspace; an id is looked
        // up there first and then everywhere, because an id names a node, not a
        // workspace.
        let starts: Vec<NodeKey> = if source_node_id.contains('/') {
            resolve_paths(&access, std::slice::from_ref(&source_node_id), Some(std::slice::from_ref(&source_workspace)))
                .await?
                .into_iter()
                .map(|n| n.key())
                .collect()
        } else {
            let here = resolve_ids(&access, std::slice::from_ref(&source_node_id), Some(std::slice::from_ref(&source_workspace))).await?;
            let found = if here.is_empty() {
                resolve_ids(&access, std::slice::from_ref(&source_node_id), None).await?
            } else {
                here
            };
            found.into_iter().map(|n| n.key()).collect()
        };

        let gate = RlsGate {
            ctx: &ctx,
            tenant_id: &tenant_id,
            repo_id: &repo_id,
            branch: &branch,
            revision: revision.as_ref(),
        };
        let mut search = NeighborhoodSearch::start(&access, &gate, &starts, &params).await?;

        let locales_to_use = get_locales_to_use(&ctx);
        let qualifier = alias.clone().unwrap_or_else(|| source_workspace.clone());

        while let Some(chunk) = search.next_chunk().await? {
            for hit in chunk {
                for locale in &locales_to_use {
                    let translated = match resolve_node_for_locale(hit.item.clone(), &ctx, locale).await? {
                        Some(n) => n,
                        None => continue,
                    };
                    let mut row = node_to_row(&translated, &qualifier, &hit.workspace, &None, &ctx, locale, None).await?;
                    insert_neighborhood_columns(&mut row, &translated, &qualifier, hit.depth, hit.via_count);
                    yield row;
                }
            }
        }
    }))
}

/// The visited-node budget a caller actually gets.
///
/// The planner accepts up to `MAX_MAX_NODES`, but only a system context or a
/// system admin may go past [`DEFAULT_MAX_NODES`]; everyone else, including a
/// query with no auth context at all, is held to the default. Holding a caller
/// to a smaller budget never truncates: the search fails at the budget.
pub(crate) fn effective_max_nodes(requested: usize, auth: Option<&AuthContext>) -> usize {
    let elevated =
        auth.is_some_and(|a| a.is_system || a.permissions().is_some_and(|p| p.is_system_admin));
    if elevated {
        requested
    } else {
        requested.min(DEFAULT_MAX_NODES)
    }
}

/// The unqualified core columns plus `depth` and `via_count`.
fn insert_neighborhood_columns(
    row: &mut Row,
    node: &Node,
    qualifier: &str,
    depth: u32,
    via_count: u64,
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
    row.insert(
        "depth".to_string(),
        PropertyValue::Integer(i64::from(depth)),
    );
    row.insert(
        format!("{qualifier}.depth"),
        PropertyValue::Integer(i64::from(depth)),
    );
    row.insert(
        "via_count".to_string(),
        PropertyValue::Integer(via_count as i64),
    );
    row.insert(
        format!("{qualifier}.via_count"),
        PropertyValue::Integer(via_count as i64),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::graph_neighborhood::MAX_MAX_NODES;
    use raisin_models::permissions::ResolvedPermissions;

    fn user(admin: bool) -> AuthContext {
        AuthContext::for_user("u").with_permissions(ResolvedPermissions {
            user_id: "u".to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![],
            is_system_admin: admin,
            home: None,
            resolved_at: None,
        })
    }

    #[test]
    fn only_a_system_or_admin_caller_may_raise_max_nodes() {
        let big = MAX_MAX_NODES;
        assert_eq!(
            effective_max_nodes(big, Some(&user(false))),
            DEFAULT_MAX_NODES
        );
        assert_eq!(effective_max_nodes(big, None), DEFAULT_MAX_NODES);
        assert_eq!(effective_max_nodes(big, Some(&user(true))), big);
        assert_eq!(effective_max_nodes(big, Some(&AuthContext::system())), big);
        // Lowering is always allowed.
        assert_eq!(effective_max_nodes(10, Some(&user(false))), 10);
    }
}
