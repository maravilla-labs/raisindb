// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Planning: resolve every target, authorize every op, detect stale
//! revisions, and predict the exact effects — before anything is written.
//!
//! Ops are planned in order against the committed state plus a small overlay
//! of what earlier ops in the same changeset did (created paths, moved
//! prefixes, deleted nodes), so "create a folder, then create inside it" and
//! "move, then patch by id" plan correctly. Authorization and root escape are
//! hard errors; a changed world (stale revision, missing target, occupied
//! destination) is a [`Conflict`] in the plan, because the caller's answer to
//! it is to re-read and re-plan, not to fix the request.

use std::collections::{HashMap, HashSet};

use raisin_agent_contract::canonical_json;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::Operation;
use raisin_storage::scope::RepoScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, ReferenceIndexRepository, Storage, WorkspaceRepository};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::changeset_types::*;
use super::root::{is_within, join, parent_of, Roots};
use super::types::*;
use super::{DevScope, NodeDevService};
use crate::services::rls_filter;

/// Referrer lookups per changeset are bounded; past this, the receipt says so
/// by listing what it found.
const MAX_REFERRER_NODES: usize = 200;
pub(super) const MAX_DESCENDANTS: u32 = 64;

#[derive(Default)]
pub(super) struct Overlay {
    pub(super) created: HashMap<(String, String), (String, String)>, // (ws, path) -> (id, type)
    pub(super) moves: Vec<(String, String, String)>,                 // (ws, old prefix, new prefix)
    pub(super) deleted: HashSet<String>,
}

impl Overlay {
    pub(super) fn current_path(&self, ws: &str, committed: &str) -> String {
        let mut p = committed.to_string();
        for (w, old, new) in &self.moves {
            if w == ws && is_within(&p, old) {
                p = format!("{new}{}", &p[old.len()..]);
            }
        }
        p
    }

    /// Map a path as it is NOW (after earlier ops) back to the committed
    /// path, or `None` when an earlier move vacated it.
    fn committed_path(&self, ws: &str, now: &str) -> Option<String> {
        let mut p = now.to_string();
        for (w, old, new) in self.moves.iter().rev() {
            if w != ws {
                continue;
            }
            if is_within(&p, new) {
                p = format!("{old}{}", &p[new.len()..]);
            } else if is_within(&p, old) {
                return None;
            }
        }
        Some(p)
    }
}

/// A resolved node: the committed node and where it is after earlier ops.
pub(super) struct Resolved {
    pub(super) node: Node,
    pub(super) path: String,
}

pub(super) fn conflict(index: usize, code: &str, message: impl Into<String>) -> Conflict {
    Conflict {
        index,
        code: code.to_string(),
        message: message.into(),
        expected: None,
        actual: None,
    }
}

pub(super) fn allow(ok: bool, what: &str) -> DevResult<()> {
    if ok {
        Ok(())
    } else {
        Err(NodeDevError::forbidden(format!(
            "permission denied: {what}"
        )))
    }
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    pub(super) async fn resolve(
        &self,
        scope: &DevScope,
        roots: &Roots,
        ov: &Overlay,
        t: &Target,
    ) -> DevResult<Option<Resolved>> {
        let (ws, path, id) = roots.target(t)?;
        let node = match (&id, &path) {
            (Some(id), _) => self.find(scope, &ws, None, Some(id), None).await?,
            (None, Some(p)) => match ov.committed_path(&ws, p) {
                Some(cp) => self.find(scope, &ws, Some(&cp), None, None).await?,
                None => None,
            },
            (None, None) => None,
        };
        Ok(node
            .filter(|n| !ov.deleted.contains(&n.id))
            .map(|n| Resolved {
                path: ov.current_path(&ws, &n.path),
                node: n,
            }))
    }

    /// Does a node (committed or created earlier in the changeset) occupy `path`?
    pub(super) async fn occupied(
        &self,
        scope: &DevScope,
        ov: &Overlay,
        ws: &str,
        path: &str,
    ) -> DevResult<bool> {
        if path == "/" || ov.created.contains_key(&(ws.to_string(), path.to_string())) {
            return Ok(true);
        }
        let Some(cp) = ov.committed_path(ws, path) else {
            return Ok(false);
        };
        Ok(self
            .find(scope, ws, Some(&cp), None, None)
            .await?
            .is_some_and(|n| !ov.deleted.contains(&n.id)))
    }

    /// The current path of a destination parent (committed, moved, or
    /// created earlier in the changeset); `None` when it does not exist.
    pub(super) async fn parent_path(
        &self,
        scope: &DevScope,
        roots: &Roots,
        ov: &Overlay,
        t: &Target,
    ) -> DevResult<Option<String>> {
        let (ws, path, id) = roots.target(t)?;
        if id.is_some() {
            return Ok(self.resolve(scope, roots, ov, t).await?.map(|r| r.path));
        }
        match path {
            Some(p) if self.occupied(scope, ov, &ws, &p).await? => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    pub(super) async fn referrers(
        &self,
        scope: &DevScope,
        ws: &str,
        ids: &[String],
    ) -> DevResult<Vec<Referrer>> {
        let workspaces = self
            .storage
            .workspaces()
            .list(RepoScope::new(&scope.tenant, &scope.repo))
            .await?;
        let mut out = Vec::new();
        for id in ids.iter().take(MAX_REFERRER_NODES) {
            for w in &workspaces {
                let found = self
                    .storage
                    .reference_index()
                    .find_referencing_nodes(scope.storage(&w.name), ws, id, false)
                    .await?;
                for (node_id, property) in found {
                    let path = self
                        .find(scope, &w.name, None, Some(&node_id), None)
                        .await?
                        .map(|n| n.path);
                    out.push(Referrer {
                        workspace: w.name.clone(),
                        node_id,
                        path,
                        property,
                        target_id: id.clone(),
                    });
                }
            }
        }
        Ok(out)
    }

    pub(super) async fn descendants(
        &self,
        scope: &DevScope,
        ws: &str,
        path: &str,
    ) -> DevResult<Vec<Node>> {
        Ok(self
            .storage
            .nodes()
            .deep_children_flat(scope.storage(ws), path, MAX_DESCENDANTS, None)
            .await?)
    }

    pub(super) fn check_revision(
        &self,
        index: usize,
        scope: &DevScope,
        ws: &str,
        node: &Node,
        expected: &Option<ExpectedRevision>,
    ) -> Option<Conflict> {
        let exp = expected.as_ref()?;
        let actual = node_revision(node);
        (actual.value != exp.value()).then(|| Conflict {
            expected: Some(exp.value().to_string()),
            actual: Some(NodeLocator::of(&scope.repo, &scope.branch, ws, node)),
            ..conflict(
                index,
                "stale_revision",
                "the node changed since it was read",
            )
        })
    }

    /// Plan a changeset. Hard errors for authorization / escape / malformed
    /// ops; conflicts for a world that changed.
    pub async fn plan(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        req: &ChangesetRequest,
    ) -> DevResult<ChangesetPlan> {
        let roots = Roots::new(req.roots.clone())?;
        if req.ops.is_empty() {
            return Err(NodeDevError::invalid("a changeset needs at least one op"));
        }
        let mut ov = Overlay::default();
        let mut ops = Vec::new();
        let mut conflicts = Vec::new();
        for (index, op) in req.ops.iter().enumerate() {
            match self
                .plan_op(scope, auth, &roots, &mut ov, index, op)
                .await?
            {
                Ok(p) => ops.push(p),
                Err(c) => conflicts.push(c),
            }
        }
        let digest = digest_of(&req.ops, &ops, &conflicts);
        Ok(ChangesetPlan {
            ops,
            conflicts,
            digest,
        })
    }

    async fn plan_op(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        ov: &mut Overlay,
        index: usize,
        op: &ChangeOp,
    ) -> DevResult<Result<PlannedOp, Conflict>> {
        let base = |action, ws: &str, node_type: &str| PlannedOp {
            index,
            action,
            workspace: ws.to_string(),
            node_id: None,
            before: None,
            after_path: None,
            node_type: node_type.to_string(),
            changed_properties: Vec::new(),
            moved_descendants: Vec::new(),
            descendants: Vec::new(),
            referrers: Vec::new(),
        };
        let loc = |ws: &str, n: &Node| NodeLocator::of(&scope.repo, &scope.branch, ws, n);
        match op {
            ChangeOp::Create {
                workspace,
                path,
                node_type,
                properties,
                ..
            } => {
                let (ws, path) = roots.resolve(workspace.as_deref(), path)?;
                let name = path.rsplit('/').next().unwrap_or_default();
                if crate::sanitize_name(name).ok().as_deref() != Some(name) {
                    return Err(NodeDevError::new(
                        422,
                        "invalid_name",
                        format!("'{name}' is not a valid node name"),
                    ));
                }
                roots.check(&ws, &path, OpKind::Create)?;
                allow(
                    rls_filter::can_create_at_path(&path, node_type, auth, &scope.permission(&ws)),
                    &format!("create {node_type} at {ws}:{path}"),
                )?;
                if self.occupied(scope, ov, &ws, &path).await? {
                    return Ok(Err(conflict(
                        index,
                        "exists",
                        format!("{ws}:{path} already exists"),
                    )));
                }
                if !self.occupied(scope, ov, &ws, &parent_of(&path)).await? {
                    return Ok(Err(conflict(
                        index,
                        "missing",
                        format!("parent of {ws}:{path} does not exist"),
                    )));
                }
                let id = nanoid::nanoid!();
                ov.created
                    .insert((ws.clone(), path.clone()), (id.clone(), node_type.clone()));
                let mut p = base(OpAction::Created, &ws, node_type);
                p.node_id = Some(id);
                p.after_path = Some(path);
                p.changed_properties = properties.keys().cloned().collect();
                Ok(Ok(p))
            }
            ChangeOp::Patch {
                target,
                expected_revision,
                set,
                unset,
                archetype,
            } => {
                let Some(r) = self.resolve(scope, roots, ov, target).await? else {
                    return Ok(Err(conflict(
                        index,
                        "missing",
                        "patch target does not exist",
                    )));
                };
                let ws = roots.target(target)?.0;
                roots.check(&ws, &r.path, OpKind::Update)?;
                allow(
                    rls_filter::can_perform(
                        &r.node,
                        Operation::Update,
                        auth,
                        &scope.permission(&ws),
                    ),
                    &format!("update {ws}:{}", r.path),
                )?;
                for k in set.keys().chain(unset.iter()) {
                    if raisin_models::nodes::is_shielded_property_key(k, &r.node.properties) {
                        return Err(NodeDevError::new(
                            422,
                            "engine_owned_property",
                            format!("'{k}' is engine-owned"),
                        ));
                    }
                }
                if let Some(c) = self.check_revision(index, scope, &ws, &r.node, expected_revision)
                {
                    return Ok(Err(c));
                }
                let mut p = base(OpAction::Updated, &ws, &r.node.node_type);
                p.node_id = Some(r.node.id.clone());
                p.before = Some(loc(&ws, &r.node));
                p.after_path = Some(r.path.clone());
                let mut keys: Vec<String> =
                    set.keys().cloned().chain(unset.iter().cloned()).collect();
                if archetype.is_some() && archetype != &r.node.archetype {
                    keys.push("archetype".into());
                }
                keys.sort();
                keys.dedup();
                p.changed_properties = keys;
                Ok(Ok(p))
            }
            ChangeOp::Move {
                target,
                to_parent,
                new_name,
                expected_revision,
            } => {
                self.plan_move(
                    scope,
                    auth,
                    roots,
                    ov,
                    index,
                    target,
                    Some(to_parent),
                    new_name.as_deref(),
                    expected_revision,
                )
                .await
            }
            ChangeOp::Rename {
                target,
                new_name,
                expected_revision,
            } => {
                self.plan_move(
                    scope,
                    auth,
                    roots,
                    ov,
                    index,
                    target,
                    None,
                    Some(new_name),
                    expected_revision,
                )
                .await
            }
            ChangeOp::Copy {
                source,
                to_parent,
                new_name,
            } => {
                let Some(r) = self.resolve(scope, roots, ov, source).await? else {
                    return Ok(Err(conflict(
                        index,
                        "missing",
                        "copy source does not exist",
                    )));
                };
                let ws = roots.target(source)?.0;
                roots.check(&ws, &r.path, OpKind::Read)?;
                let parent = roots.target(to_parent)?;
                if parent.0 != ws {
                    return Err(NodeDevError::invalid("copy stays inside one workspace"));
                }
                let Some(parent_path) = self.parent_path(scope, roots, ov, to_parent).await? else {
                    return Ok(Err(conflict(
                        index,
                        "missing",
                        "copy destination parent does not exist",
                    )));
                };
                let name = new_name.clone().unwrap_or_else(|| r.node.name.clone());
                let dest = join(&parent_path, &name);
                roots.check(&ws, &dest, OpKind::Copy)?;
                let perm = scope.permission(&ws);
                allow(
                    rls_filter::can_perform(&r.node, Operation::Read, auth, &perm),
                    "read the copy source",
                )?;
                allow(
                    rls_filter::can_create_at_path(&dest, &r.node.node_type, auth, &perm),
                    &format!("create at {ws}:{dest}"),
                )?;
                if self.occupied(scope, ov, &ws, &dest).await? {
                    return Ok(Err(conflict(
                        index,
                        "exists",
                        format!("{ws}:{dest} already exists"),
                    )));
                }
                let mut p = base(OpAction::Copied, &ws, &r.node.node_type);
                p.node_id = None;
                p.before = Some(NodeLocator {
                    path: r.path.clone(),
                    ..loc(&ws, &r.node)
                });
                p.after_path = Some(dest);
                p.descendants = self
                    .descendants(scope, &ws, &r.node.path)
                    .await?
                    .iter()
                    .map(|d| loc(&ws, d))
                    .collect();
                Ok(Ok(p))
            }
            ChangeOp::Delete {
                target,
                expected_revision,
                recursive,
            } => {
                let Some(r) = self.resolve(scope, roots, ov, target).await? else {
                    return Ok(Err(conflict(
                        index,
                        "missing",
                        "delete target does not exist",
                    )));
                };
                let ws = roots.target(target)?.0;
                roots.check(&ws, &r.path, OpKind::Delete)?;
                let perm = scope.permission(&ws);
                let desc = self.descendants(scope, &ws, &r.node.path).await?;
                if !desc.is_empty() && !recursive {
                    return Ok(Err(conflict(
                        index,
                        "not_empty",
                        "node has children; pass recursive: true",
                    )));
                }
                for n in std::iter::once(&r.node).chain(desc.iter()) {
                    allow(
                        rls_filter::can_perform(n, Operation::Delete, auth, &perm),
                        &format!("delete {ws}:{}", n.path),
                    )?;
                }
                if let Some(c) = self.check_revision(index, scope, &ws, &r.node, expected_revision)
                {
                    return Ok(Err(c));
                }
                let ids: Vec<String> = std::iter::once(&r.node)
                    .chain(desc.iter())
                    .map(|n| n.id.clone())
                    .collect();
                let mut p = base(OpAction::Deleted, &ws, &r.node.node_type);
                p.node_id = Some(r.node.id.clone());
                p.before = Some(loc(&ws, &r.node));
                p.descendants = desc.iter().map(|d| loc(&ws, d)).collect();
                p.referrers = self
                    .referrers(scope, &ws, &ids)
                    .await?
                    .into_iter()
                    .filter(|x| !ids.contains(&x.node_id))
                    .collect();
                ov.deleted.extend(ids);
                Ok(Ok(p))
            }
        }
    }
}

/// The review digest: the ops as requested, and the exact node revisions and
/// ids they were planned against. Any change to either changes it.
pub(crate) fn digest_of(ops: &[ChangeOp], planned: &[PlannedOp], conflicts: &[Conflict]) -> String {
    let basis: Vec<serde_json::Value> = planned
        .iter()
        .map(|p| {
            json!({
                "i": p.index,
                "id": p.node_id,
                "rev": p.before.as_ref().and_then(|b| b.revision.as_ref()).map(|r| &r.value),
                "to": p.after_path,
            })
        })
        .collect();
    let codes: Vec<(usize, &str)> = conflicts
        .iter()
        .map(|c| (c.index, c.code.as_str()))
        .collect();
    // Create ids are minted per plan, so they are left out of the digest:
    // re-planning the same request against the same world digests the same.
    let basis: Vec<serde_json::Value> = basis
        .into_iter()
        .zip(planned)
        .map(|(mut b, p)| {
            if p.action == OpAction::Created {
                b["id"] = serde_json::Value::Null;
            }
            b
        })
        .collect();
    let doc = json!({ "ops": ops, "basis": basis, "conflicts": codes });
    let mut h = Sha256::new();
    h.update(canonical_json(&doc).as_bytes());
    format!("sha256:{}", hex(&h.finalize()))
}
