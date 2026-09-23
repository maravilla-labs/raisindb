// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Applying a conflict-free plan in ONE transaction, together with the
//! changeset record, and building the exact receipt afterwards.

use std::collections::HashMap;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{BranchRepository, NodeRepository, Storage};

use super::changeset_store::{begin, stage_record};
use super::changeset_types::*;
use super::root::parent_of;
use super::types::*;
use super::{DevScope, NodeDevService};
use crate::services::transaction::Transaction;

fn to_property(key: &str, v: &serde_json::Value) -> DevResult<PropertyValue> {
    serde_json::from_value(v.clone())
        .map_err(|e| NodeDevError::new(422, "invalid_property", format!("'{key}': {e}")))
}

fn new_node(ws: &str, id: &str, path: &str, op: &ChangeOp, actor: &str) -> DevResult<Node> {
    let ChangeOp::Create {
        node_type,
        archetype,
        properties,
        ..
    } = op
    else {
        return Err(NodeDevError::new(500, "internal", "not a create"));
    };
    let mut props = HashMap::new();
    for (k, v) in properties {
        props.insert(k.clone(), to_property(k, v)?);
    }
    let name = path.rsplit('/').next().unwrap_or_default().to_string();
    Ok(Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        node_type: node_type.clone(),
        archetype: archetype.clone(),
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Node::extract_parent_name_from_path(path),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        created_by: Some(actor.to_string()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        translations: None,
        tenant_id: None,
        workspace: Some(ws.to_string()),
        owner_id: None,
        relations: Vec::new(),
    })
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    async fn apply_op(
        &self,
        ctx: &dyn TransactionalContext,
        scope: &DevScope,
        actor: &str,
        op: &ChangeOp,
        p: &PlannedOp,
        copies: &mut HashMap<usize, String>,
    ) -> DevResult<()> {
        let ws = p.workspace.as_str();
        let id = p.node_id.clone().unwrap_or_default();
        match op {
            ChangeOp::Create { .. } => {
                let path = p.after_path.clone().unwrap_or_default();
                let node = new_node(ws, &id, &path, op, actor)?;
                ctx.add_node(ws, &node).await?;
                Transaction::new(
                    self.storage.clone(),
                    scope.tenant.clone(),
                    scope.repo.clone(),
                    scope.branch.clone(),
                    ws.to_string(),
                )
                .create_initial_structure_children(ctx, &node, actor)
                .await?;
            }
            ChangeOp::Patch {
                set,
                unset,
                archetype,
                ..
            } => {
                let mut node = ctx
                    .get_node(ws, &id)
                    .await?
                    .ok_or_else(|| NodeDevError::not_found(format!("node {id} vanished")))?;
                for (k, v) in set {
                    node.properties.insert(k.clone(), to_property(k, v)?);
                }
                for k in unset {
                    node.properties.remove(k);
                }
                if archetype.is_some() && archetype != &node.archetype {
                    node.archetype = archetype.clone();
                }
                ctx.put_node(ws, &node).await?;
            }
            ChangeOp::Move { .. } | ChangeOp::Rename { .. } => {
                let dest = p.after_path.clone().unwrap_or_default();
                ctx.move_node_tree(ws, &id, &dest).await?;
            }
            ChangeOp::Copy { .. } => {
                let src = p
                    .before
                    .as_ref()
                    .map(|b| b.path.clone())
                    .unwrap_or_default();
                let dest = p.after_path.clone().unwrap_or_default();
                let name = dest.rsplit('/').next().map(str::to_string);
                let copied = ctx
                    .copy_node_tree(ws, &src, &parent_of(&dest), name.as_deref(), actor)
                    .await?;
                copies.insert(p.index, copied.id);
            }
            ChangeOp::Delete { .. } => {
                // Deepest first: a transaction delete does not cascade.
                let mut desc: Vec<&NodeLocator> = p.descendants.iter().collect();
                desc.sort_by_key(|d| std::cmp::Reverse(d.path.matches('/').count()));
                for d in desc {
                    if let Some(did) = &d.node_id {
                        ctx.delete_node(ws, did).await?;
                    }
                }
                ctx.delete_node(ws, &id).await?;
            }
        }
        Ok(())
    }

    /// Apply `plan` (conflict-free) and stage `record` (already marked
    /// committed, carrying the skeleton receipt) in the same transaction.
    /// Returns the ids minted by copies, keyed by op index.
    pub(crate) async fn apply_plan(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        req: &ChangesetRequest,
        plan: &ChangesetPlan,
        record: &ChangesetRecord,
    ) -> DevResult<HashMap<usize, String>> {
        let message = req
            .message
            .clone()
            .unwrap_or_else(|| format!("changeset {}", record.changeset_id));
        let ctx = begin(self.storage.as_ref(), scope, auth, &message).await?;
        let actor = auth.principal_id().unwrap_or_else(|| auth.actor_id());
        let mut copies = HashMap::new();
        for (op, p) in req.ops.iter().zip(plan.ops.iter()) {
            if let Err(e) = self
                .apply_op(ctx.as_ref(), scope, &actor, op, p, &mut copies)
                .await
            {
                let _ = ctx.rollback().await;
                return Err(e);
            }
        }
        if let Err(e) = stage_record(ctx.as_ref(), record).await {
            let _ = ctx.rollback().await;
            return Err(e);
        }
        ctx.commit().await?;
        Ok(copies)
    }

    async fn now(&self, scope: &DevScope, ws: &str, id: &str) -> DevResult<Option<NodeLocator>> {
        Ok(self
            .storage
            .nodes()
            .get(scope.storage(ws), id, None)
            .await?
            .map(|n| NodeLocator::of(&scope.repo, &scope.branch, ws, &n)))
    }

    /// The receipt, read back from what actually committed.
    pub(crate) async fn build_receipt(
        &self,
        scope: &DevScope,
        changeset_id: &str,
        plan: &ChangesetPlan,
        copies: &HashMap<usize, String>,
    ) -> DevResult<Receipt> {
        let head = self
            .storage
            .branches()
            .get_branch(&scope.tenant, &scope.repo, &scope.branch)
            .await?
            .map(|b| b.head.to_string());
        let mut ops = Vec::with_capacity(plan.ops.len());
        for p in &plan.ops {
            let ws = p.workspace.as_str();
            let mut r = skeleton(scope, p);
            match p.action {
                OpAction::Created | OpAction::Updated | OpAction::Moved => {
                    if let Some(id) = &p.node_id {
                        r.new = self.now(scope, ws, id).await?;
                    }
                    for m in &mut r.moved_descendants {
                        if let Some(id) = &m.to.node_id {
                            if let Some(l) = self.now(scope, ws, id).await? {
                                m.to = l;
                            }
                        }
                    }
                }
                OpAction::Copied => {
                    if let Some(id) = copies.get(&p.index) {
                        r.new = self.now(scope, ws, id).await?;
                    }
                    if let Some(root) = &r.new {
                        r.created_descendants = self
                            .storage
                            .nodes()
                            .deep_children_flat(scope.storage(ws), &root.path, 64, None)
                            .await?
                            .iter()
                            .map(|n| NodeLocator::of(&scope.repo, &scope.branch, ws, n))
                            .collect();
                    }
                }
                OpAction::Deleted => {}
            }
            ops.push(r);
        }
        Ok(Receipt {
            changeset_id: changeset_id.to_string(),
            repository: scope.repo.clone(),
            branch: scope.branch.clone(),
            committed_revision: head,
            ops,
            replayed: false,
        })
    }
}

/// What a receipt says before post-commit reads (stored with the commit).
pub(crate) fn skeleton(scope: &DevScope, p: &PlannedOp) -> OpReceipt {
    let new = p.after_path.as_ref().map(|path| NodeLocator {
        repository: scope.repo.clone(),
        branch: scope.branch.clone(),
        workspace: p.workspace.clone(),
        path: path.clone(),
        node_id: p.node_id.clone(),
        revision: None,
    });
    OpReceipt {
        index: p.index,
        action: p.action,
        old: p.before.clone(),
        new: if p.action == OpAction::Deleted {
            None
        } else {
            new
        },
        changed_properties: p.changed_properties.clone(),
        created_descendants: Vec::new(),
        deleted_descendants: if p.action == OpAction::Deleted {
            p.descendants.clone()
        } else {
            Vec::new()
        },
        moved_descendants: p.moved_descendants.clone(),
        rewritten_references: if p.action == OpAction::Moved {
            p.referrers.clone()
        } else {
            Vec::new()
        },
    }
}
