//! Deciding a target: read, translate, permit — before it is descended into.
//!
//! THE ONE PLACE A REFERENCED NODE IS READ. Every decision a target needs is
//! made here, once, and recorded in the memo: whether it exists at the
//! statement's snapshot, whether it is hidden in the statement's locale, and
//! whether the caller may read it. All three negative answers are the same
//! `None`, so the bare reference RESOLVE leaves behind says nothing about which
//! one it was.
//!
//! Reads are one node at a time through the storage trait today. The miss
//! loop in [`ReferenceResolver::read_targets`] is the seam a batched
//! `get_many_for_read` replaces: it already receives a whole frontier level.

use super::memo::{MemoKey, ReadScope, Target};
use super::walk::TargetRef;
use super::ReferenceResolver;
use crate::services::rls_filter;
use crate::services::translation_resolver::TranslationResolver;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{BranchScope, NodeRepository, Storage, StorageScope};
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
            let entry = node.map(|node| Arc::new(Target::from_node(&node, fields)));
            self.memo.store(read, target, entry.clone());
            out[i] = Some(entry);
        }

        Ok(out.into_iter().map(Option::flatten).collect())
    }

    /// Decide each target: `Some(node)` to inline, `None` to leave bare.
    async fn read_targets(&self, targets: &[&TargetRef]) -> Result<Vec<Option<Node>>> {
        let mut out = Vec::with_capacity(targets.len());
        for target in targets {
            out.push(self.read_target(target).await?);
        }
        Ok(out)
    }

    async fn read_target(&self, target: &TargetRef) -> Result<Option<Node>> {
        let permission_scope = PermissionScope::new(&target.workspace, &self.branch);

        // Decided without reading the node when the answer is certain: a caller
        // with no read grant for the workspace (or, for a path reference, for
        // that path) is denied whatever the node turns out to be. Skipped under
        // graph RLS — the full check below is the one that sees the graph.
        if let Some(auth) = &self.auth {
            let path = target.is_path().then_some(target.locator.as_str());
            if !auth.uses_graph_rls() && !rls_filter::may_read(auth, &permission_scope, path) {
                return Ok(None);
            }
        }

        self.memo.count_read();
        let scope = StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &target.workspace,
        );
        let node = if target.is_path() {
            self.storage
                .nodes()
                .get_by_path(scope, &target.locator, Some(&self.snapshot))
                .await?
        } else {
            self.storage
                .nodes()
                .get(scope, &target.locator, Some(&self.snapshot))
                .await?
        };
        let Some(node) = node else {
            return Ok(None);
        };

        // Translate BEFORE row-level security, so a field filter applies to the
        // values actually inlined — an overlay merged afterwards could put back
        // a field the caller is not allowed to see.
        let Some(node) = self.translate(&target.workspace, node).await? else {
            return Ok(None);
        };

        let Some(auth) = &self.auth else {
            return Ok(Some(node));
        };
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
