//! Deciding a target: read, translate, permit — before it is descended into.
//!
//! THE ONE PLACE A REFERENCED NODE IS READ. Every decision a target needs is
//! made here, once, and recorded in the memo: whether it exists at the
//! statement's snapshot, whether it is hidden in the statement's locale, and
//! whether the caller may read it. All three negative answers are the same
//! `None`, so the bare reference RESOLVE leaves behind says nothing about which
//! one it was.
//!
//! A whole frontier level is read in one batched storage call
//! (`get_many_for_read`), at the statement's snapshot revision and through its
//! storage snapshot, so a level costs one blocking task and one iterator per
//! column family rather than one await per target.

use super::memo::{MemoKey, ReadScope, Target};
use super::walk::TargetRef;
use super::ReferenceResolver;
use crate::services::rls_filter;
use crate::services::translation_resolver::TranslationResolver;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{BatchReadItem, BranchScope, NodeRepository, ReadOpts, Storage, StorageScope};
use std::sync::Arc;

impl<S: Storage> ReferenceResolver<S> {
    /// Every target of one frontier level, in order. Memo hits cost nothing;
    /// the misses are decided together.
    pub(super) async fn fetch_all(
        &self,
        read: &Arc<ReadScope>,
        targets: &[TargetRef],
        fields: Option<&[String]>,
    ) -> Result<Vec<Option<Arc<Target>>>> {
        let mut out: Vec<Option<Option<Arc<Target>>>> = Vec::with_capacity(targets.len());
        let mut misses: Vec<(usize, &TargetRef)> = Vec::new();
        for (i, target) in targets.iter().enumerate() {
            match self.memo.lookup(&MemoKey::new(read, target)) {
                Some(entry) => out.push(Some(entry)),
                None => {
                    self.memo.admit_target()?;
                    misses.push((i, target));
                    out.push(None);
                }
            }
        }

        let wanted: Vec<&TargetRef> = misses.iter().map(|(_, t)| *t).collect();
        let nodes = self.read_targets(&wanted).await?;
        for ((i, target), node) in misses.into_iter().zip(nodes) {
            let entry = node.map(|node| Arc::new(Target::from_node(node, fields)));
            self.memo.store(read, target, entry.clone());
            out[i] = Some(entry);
        }

        Ok(out.into_iter().map(Option::flatten).collect())
    }

    /// Decide each target: `Some(node)` to inline, `None` to leave bare.
    ///
    /// The whole level is read in ONE batched call
    /// (`NodeRepository::get_many_for_read`, through the statement's storage
    /// snapshot when it has one) — unless `sql.batched_fetch` is off, which
    /// reads one target at a time as before. Either way each target is then
    /// translated and permitted on its own.
    async fn read_targets(&self, targets: &[&TargetRef]) -> Result<Vec<Option<Node>>> {
        let mut out: Vec<Option<Node>> = vec![None; targets.len()];
        let wanted: Vec<usize> = (0..targets.len())
            .filter(|&i| !self.denied_before_read(targets[i]))
            .collect();
        for _ in &wanted {
            self.memo.count_read();
        }

        let fetched = if self.batched_fetch {
            let items: Vec<BatchReadItem> = wanted
                .iter()
                .map(|&i| {
                    let target = targets[i];
                    if target.is_path() {
                        BatchReadItem::path(&target.workspace, &target.locator)
                    } else {
                        BatchReadItem::id(&target.workspace, &target.locator)
                    }
                })
                .collect();
            self.storage
                .nodes()
                .get_many_for_read(
                    BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                    &items,
                    &self.snapshot,
                    self.read_snapshot.as_ref(),
                    ReadOpts::default(),
                )
                .await?
        } else {
            let mut fetched = Vec::with_capacity(wanted.len());
            for &i in &wanted {
                fetched.push(self.read_one(targets[i]).await?);
            }
            fetched
        };

        for (i, node) in wanted.into_iter().zip(fetched) {
            if let Some(node) = node {
                out[i] = self.admit(&targets[i].workspace, node).await?;
            }
        }
        Ok(out)
    }

    /// Decided without reading the node when the answer is certain: a caller
    /// with no read grant for the workspace (or, for a path reference, for
    /// that path) is denied whatever the node turns out to be. Skipped under
    /// graph RLS — the full check in [`Self::admit`] is the one that sees the
    /// graph.
    fn denied_before_read(&self, target: &TargetRef) -> bool {
        let Some(auth) = &self.auth else {
            return false;
        };
        let permission_scope = PermissionScope::new(&target.workspace, &self.branch);
        let path = target.is_path().then_some(target.locator.as_str());
        !auth.uses_graph_rls() && !rls_filter::may_read(auth, &permission_scope, path)
    }

    /// One target, unbatched (`sql.batched_fetch = false`).
    async fn read_one(&self, target: &TargetRef) -> Result<Option<Node>> {
        let scope = StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &target.workspace,
        );
        if target.is_path() {
            self.storage
                .nodes()
                .get_by_path(scope, &target.locator, Some(&self.snapshot))
                .await
        } else {
            self.storage
                .nodes()
                .get(scope, &target.locator, Some(&self.snapshot))
                .await
        }
    }

    /// A read target, translated and permitted; `None` to leave it bare.
    async fn admit(&self, workspace: &str, node: Node) -> Result<Option<Node>> {
        // Translate BEFORE row-level security, so a field filter applies to the
        // values actually inlined — an overlay merged afterwards could put back
        // a field the caller is not allowed to see.
        let Some(node) = self.translate(workspace, node).await? else {
            return Ok(None);
        };

        let Some(auth) = &self.auth else {
            return Ok(Some(node));
        };
        let permission_scope = PermissionScope::new(workspace, &self.branch);
        Ok(rls_filter::filter_node_with_graph(
            &*self.storage,
            node,
            auth,
            &permission_scope,
            BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
            &self.snapshot,
        )
        .await)
    }

    /// The node in the resolution's language; `None` when it is hidden there.
    ///
    /// The referenced node's OWN workspace, not the reader's — a page in
    /// `stories` referencing a contact in `people` has its translations stored
    /// against `people`.
    async fn translate(&self, workspace: &str, node: Node) -> Result<Option<Node>> {
        let Some(resolution) = self.effective_locale() else {
            return Ok(Some(node));
        };
        let resolver = match &self.translations {
            Some(resolver) => resolver.clone(),
            None => Arc::new(TranslationResolver::new(
                Arc::new(self.storage.translations().clone()),
                resolution.config.clone(),
            )),
        };
        resolver
            .resolve_node(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                workspace,
                node,
                &resolution.locale,
                &self.snapshot,
            )
            .await
    }
}
