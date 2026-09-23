// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Planning a move or rename: the destination, the subtree it carries, the
//! references that point into it, and the checks a move must pass (not into
//! its own subtree, not onto an occupied path, not a published node).

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::Operation;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

use super::changeset_plan::{allow, conflict, Overlay};
use super::changeset_types::*;
use super::root::{is_within, join, parent_of, Roots};
use super::types::*;
use super::{DevScope, NodeDevService};
use crate::services::rls_filter;

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn plan_move(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        roots: &Roots,
        ov: &mut Overlay,
        index: usize,
        target: &Target,
        to_parent: Option<&Target>,
        new_name: Option<&str>,
        expected: &Option<ExpectedRevision>,
    ) -> DevResult<Result<PlannedOp, Conflict>> {
        let Some(r) = self.resolve(scope, roots, ov, target).await? else {
            return Ok(Err(conflict(
                index,
                "missing",
                "move target does not exist",
            )));
        };
        let ws = roots.target(target)?.0;
        let parent_path = match to_parent {
            None => parent_of(&r.path),
            Some(t) => {
                let tp = roots.target(t)?;
                if tp.0 != ws {
                    return Err(NodeDevError::invalid("a move stays inside one workspace"));
                }
                match self.parent_path(scope, roots, ov, t).await? {
                    Some(p) => p,
                    None => {
                        return Ok(Err(conflict(
                            index,
                            "missing",
                            "move destination parent does not exist",
                        )))
                    }
                }
            }
        };
        let name = match new_name {
            Some(n) => crate::sanitize_name(n)?,
            None => r.node.name.clone(),
        };
        let dest = join(&parent_path, &name);
        roots.check(&ws, &r.path, OpKind::Move)?;
        roots.check(&ws, &dest, OpKind::Create)?;
        let perm = scope.permission(&ws);
        allow(
            rls_filter::can_perform(&r.node, Operation::Update, auth, &perm),
            "move the node",
        )?;
        allow(
            rls_filter::can_create_at_path(&dest, &r.node.node_type, auth, &perm),
            &format!("create at {ws}:{dest}"),
        )?;
        if dest == r.path {
            return Err(NodeDevError::invalid("the move does not change the path"));
        }
        if is_within(&dest, &r.path) {
            return Ok(Err(conflict(
                index,
                "invalid_move",
                "cannot move a node into its own subtree",
            )));
        }
        if self.occupied(scope, ov, &ws, &dest).await? {
            return Ok(Err(conflict(
                index,
                "exists",
                format!("{ws}:{dest} already exists"),
            )));
        }
        if let Some(c) = self.check_revision(index, scope, &ws, &r.node, expected) {
            return Ok(Err(c));
        }
        let desc = self.descendants(scope, &ws, &r.node.path).await?;
        if let Some(p) = std::iter::once(&r.node)
            .chain(desc.iter())
            .find(|n| n.published_at.is_some())
        {
            return Ok(Err(conflict(
                index,
                "published",
                format!("{} is published; unpublish before moving", p.path),
            )));
        }
        let ids: Vec<String> = std::iter::once(&r.node)
            .chain(desc.iter())
            .map(|n| n.id.clone())
            .collect();
        let loc = |n: &Node, path: String| NodeLocator {
            path,
            revision: None,
            ..NodeLocator::of(&scope.repo, &scope.branch, &ws, n)
        };
        let from_root = r.path.clone();
        let moved = desc
            .iter()
            .map(|d| {
                let now = ov.current_path(&ws, &d.path);
                MovedEntry {
                    to: loc(d, format!("{dest}{}", &now[from_root.len()..])),
                    from_path: now,
                }
            })
            .collect();
        ov.moves.push((ws.clone(), from_root.clone(), dest.clone()));
        Ok(Ok(PlannedOp {
            index,
            action: OpAction::Moved,
            workspace: ws.clone(),
            node_id: Some(r.node.id.clone()),
            before: Some(NodeLocator {
                path: from_root,
                ..NodeLocator::of(&scope.repo, &scope.branch, &ws, &r.node)
            }),
            after_path: Some(dest),
            node_type: r.node.node_type.clone(),
            changed_properties: Vec::new(),
            moved_descendants: moved,
            descendants: Vec::new(),
            referrers: self.referrers(scope, &ws, &ids).await?,
        }))
    }
}
