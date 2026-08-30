//! Cross-branch node-set copy (branch promotion primitive).
//!
//! Copies a set of nodes — optionally with all descendants — from a source
//! branch onto a target branch in ONE atomic WriteBatch, **preserving node
//! ids** so repeated promotions update the same target nodes. Optionally
//! prunes target-branch nodes that no longer exist in the copied source set
//! (`delete_missing`), turning the operation into a one-way branch sync.
//!
//! Index maintenance mirrors the single-branch write paths:
//! - new/changed nodes go through `add_node_to_batch_with_parent_id`
//!   (node blob + PATH / NODE_PATH / PROPERTY / REFERENCE / RELATION /
//!   ORDERED_CHILDREN entries at the copy revision),
//! - pre-existing target nodes get old-value compound/unique tombstones
//!   first (the `update_impl` stale-index pattern), plus stale PATH and
//!   ORDERED_CHILDREN tombstones when the node moved on the source,
//! - pruned nodes get the full `delete_impl` tombstone set.

mod capture;
mod collect;
mod events;
mod helpers;
mod prune;
mod relations;
mod roots;
mod schema;
mod secrets;
mod stage;
mod translations;

use super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::TranslationMeta;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{
    BranchRepository, CrossBranchCopySummary, CrossBranchNodeChange, NodeChangeInfo,
    RevisionRepository,
};
use rocksdb::WriteBatch;
use std::collections::{HashMap, HashSet};

/// Immutable per-operation context threaded through the staging submodule.
struct CopyScope<'a> {
    tenant_id: &'a str,
    repo_id: &'a str,
    source_branch: &'a str,
    target_branch: &'a str,
    workspace: &'a str,
    revision: &'a HLC,
    /// Upper bound for reads of the SOURCE branch: the pin, else its head.
    source_max_revision: &'a HLC,
    now: chrono::DateTime<chrono::Utc>,
    meta_actor: &'a str,
    meta_message: &'a str,
    meta_is_system: bool,
}

/// Mutable results accumulated while staging entries.
struct CopyAccumulators {
    changes: Vec<CrossBranchNodeChange>,
    change_infos: Vec<NodeChangeInfo>,
    /// (node as written, target parent id, order label) for replication capture
    nodes_for_replication: Vec<(Node, String, String)>,
    /// Highest order label written per target parent (metadata cache upkeep).
    max_label_per_parent: HashMap<String, String>,
    /// Secret versions copied into the target branch, for replication capture.
    secrets: Vec<crate::secret_store::StoredSecret>,
    /// Edge deltas the promotion produced on the target, for replication.
    relation_ops: Vec<raisin_replication::OpType>,
    /// Ids of target nodes the copy rewrote with byte-identical content.
    ///
    /// The row is still written (new revision, refreshed indexes — the copy
    /// stays self-healing), but nothing OBSERVABLE changed, so no node event is
    /// published for it. Without this a steady-state re-publish would re-embed
    /// every unchanged node on every run. Mirrors `track_update`'s no-op guard
    /// on the transaction path; see `events.rs`.
    content_unchanged: HashSet<String>,
}

/// One source node staged for copying, with its resolved parent ids.
struct CopyEntry {
    node: Node,
    /// Parent id on the source branch (for reading the source order label)
    src_parent_id: String,
    /// Parent id on the target branch ("/" for root-level nodes; ids are
    /// preserved, so for non-root depths this equals the source parent id)
    dst_parent_id: String,
}

/// A resolved copy root: the source node plus its parent on both branches.
struct RootContext {
    node: Node,
    src_parent_id: String,
    dst_parent_id: String,
}

impl NodeRepositoryImpl {
    /// Copy a node set across branches — see
    /// [`raisin_storage::NodeRepository::copy_nodes_across_branches`] for the
    /// contract. All writes happen in a single WriteBatch under one revision;
    /// the target branch HEAD advances to that revision atomically.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn copy_nodes_across_branches_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        roots: &[String],
        recursive: bool,
        delete_missing: bool,
        source_revision: Option<&raisin_hlc::HLC>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<CrossBranchCopySummary> {
        let (mut target, root_ctxs) = self
            .resolve_cross_branch_roots(
                tenant_id,
                repo_id,
                source_branch,
                target_branch,
                workspace,
                roots,
                source_revision,
            )
            .await?;

        // ========== STEP 1: collect the source node set (BFS, parents first) ==========
        let (entries, src_ids) = self.collect_cross_branch_entries(
            tenant_id,
            repo_id,
            source_branch,
            workspace,
            &root_ctxs,
            recursive,
            source_revision,
        )?;

        // ========== STEP 2: carry the schema the node set references ==========
        //
        // Content promoted onto a branch that has no definition for its
        // node_type is indexed by the planner's legacy fallback (per-field
        // VECTOR/FULLTEXT selection silently discarded) while a `__branch` SQL
        // write onto the same branch is REFUSED. Carrying the schema is what
        // makes the two legs agree; see `schema.rs` for why the fix is here and
        // not in either policy.
        //
        // Deliberately BEFORE the copy revision is allocated: these upserts are
        // separate transactions with their own revisions, so running them first
        // keeps the schema strictly older than the content that needs it and
        // leaves the target HEAD monotonic. It is best-effort — a name that
        // does not resolve on the source is skipped, never fatal.
        let carried_schema = self
            .carry_schema_to_target(
                &entries,
                tenant_id,
                repo_id,
                source_branch,
                target_branch,
                source_revision,
                operation_meta
                    .as_ref()
                    .map(|m| m.actor.as_str())
                    .unwrap_or("system"),
                operation_meta.as_ref().map(|m| m.is_system).unwrap_or(true),
            )
            .await?;
        if carried_schema.wrote_anything() {
            // The carry advanced the target HEAD, so the `target` snapshot
            // taken above is stale and `rev_meta.parent` would name a revision
            // that is no longer the tip. Re-read it.
            if let Some(refreshed) = self
                .branch_repo
                .get_branch(tenant_id, repo_id, target_branch)
                .await?
            {
                target = refreshed;
            }
        }

        // ========== STEP 3: single revision for the whole operation ==========
        let revision = self.revision_repo.allocate_revision();

        // ========== STEP 4: build the single WriteBatch ==========
        let mut batch = WriteBatch::default();
        let now = chrono::Utc::now();

        let changes: Vec<CrossBranchNodeChange> = Vec::new();
        let change_infos: Vec<NodeChangeInfo> = Vec::new();
        // (node as written, target parent id, order label) for replication capture
        let nodes_for_replication: Vec<(Node, String, String)> = Vec::new();
        // Highest order label written per target parent (metadata cache upkeep).
        let max_label_per_parent: HashMap<String, String> = HashMap::new();

        let (meta_actor, meta_message, meta_is_system) = if let Some(meta) = operation_meta.as_ref()
        {
            (meta.actor.clone(), meta.message.clone(), meta.is_system)
        } else {
            (
                "system".to_string(),
                format!(
                    "Copy {} node set(s) from branch '{}' to '{}'",
                    roots.len(),
                    source_branch,
                    target_branch
                ),
                true,
            )
        };

        // Edges are read from the source at the same bound as the nodes were:
        // the pin when there is one, otherwise the source head.
        let source_head = self
            .branch_repo
            .get_branch(tenant_id, repo_id, source_branch)
            .await?
            .map(|b| b.head)
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source branch '{}' not found",
                    source_branch
                ))
            })?;
        let source_max_revision = source_revision.copied().unwrap_or(source_head);

        let scope = CopyScope {
            tenant_id,
            repo_id,
            source_branch,
            target_branch,
            workspace,
            revision: &revision,
            source_max_revision: &source_max_revision,
            now,
            meta_actor: &meta_actor,
            meta_message: &meta_message,
            meta_is_system,
        };
        let mut acc = CopyAccumulators {
            changes,
            change_infos,
            nodes_for_replication,
            max_label_per_parent,
            secrets: Vec::new(),
            relation_ops: Vec::new(),
            content_unchanged: HashSet::new(),
        };
        for entry in &entries {
            self.stage_cross_branch_entry(&mut batch, entry, &scope, &mut acc)
                .await?;
        }
        let CopyAccumulators {
            mut changes,
            mut change_infos,
            nodes_for_replication,
            max_label_per_parent,
            secrets,
            relation_ops,
            content_unchanged,
        } = acc;

        let copied = entries.len();

        // Last-child metadata cache: only advance it — writing a smaller
        // label would make the next append mint an already-used label.
        {
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            for (parent_id, label) in &max_label_per_parent {
                let current = self.get_last_order_label(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    parent_id,
                )?;
                if current
                    .as_deref()
                    .map(|c| label.as_str() > c)
                    .unwrap_or(true)
                {
                    let metadata_key = keys::last_child_metadata_key(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        parent_id,
                    );
                    batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
                }
            }
        }

        // ========== STEP 5: delete_missing — prune target-only nodes ==========
        let deleted_ids = if delete_missing {
            self.prune_missing_targets(
                &mut batch,
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &revision,
                &root_ctxs,
                &src_ids,
                &mut changes,
                &mut change_infos,
            )
            .await?
        } else {
            HashSet::new()
        };
        let deleted = deleted_ids.len();

        // ========== STEP 6: revision index + branch HEAD in the same batch ==========
        for change in &changes {
            self.revision_repo.index_node_change_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                &revision,
                &change.node_id,
            )?;
        }

        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, target_branch, revision)
            .await?;

        // ========== STEP 7: atomic commit ==========
        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Atomic cross-branch copy failed: {}", e))
        })?;

        tracing::info!(
            "copy_nodes_across_branches: {} copied, {} pruned, {} -> {} at revision {}",
            copied,
            deleted,
            source_branch,
            target_branch,
            revision
        );

        // ========== STEP 8: replication + revision metadata (post-commit) ==========
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                target_branch,
                &updated_branch,
                revision,
            )
            .await;

        // Secrets FIRST, on the same (tenant, repo) lane as the node operations
        // below: `OperationCapture` keeps one vector clock per (tenant, repo),
        // so same-lane-and-earlier is what makes a peer's causal delivery buffer
        // hold the node snapshot until the secret has landed.
        self.capture_secret_versions(tenant_id, repo_id, target_branch, &meta_actor, &secrets)
            .await;

        self.capture_cross_branch_operations(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &meta_actor,
            &revision,
            &nodes_for_replication,
            &deleted_ids,
        )
        .await;

        // Edge deltas AFTER the node snapshots, same lane: a peer applies the
        // AddRelation once both endpoints' snapshots have landed.
        for op_type in relation_ops {
            let _ = self
                .operation_capture
                .capture_operation_with_revision(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    target_branch.to_string(),
                    op_type,
                    meta_actor.clone(),
                    None,
                    meta_is_system,
                    Some(revision),
                )
                .await;
        }

        // Derived state (embeddings, fulltext, spatial, triggers, WS
        // subscribers) is driven ENTIRELY by node events, and this path never
        // touches the transaction commit that normally emits them. Emitting here
        // rather than in each caller is what makes the WS handler and the
        // function binding — i.e. Studio publish — behave identically. See
        // `events.rs` for the full rationale and the no-op suppression.
        self.emit_cross_branch_events(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &revision,
            &meta_actor,
            &changes,
            &content_unchanged,
        );

        // Always store revision metadata — this is what makes the promotion
        // visible to branch diffs and change tracking.
        let operation = operation_meta.map(|mut op_meta| {
            op_meta.revision = revision;
            if op_meta.node_id.is_empty() {
                if let Some(rc) = root_ctxs.first() {
                    op_meta.node_id = rc.node.id.clone();
                }
            }
            op_meta
        });

        let rev_meta = raisin_storage::RevisionMeta {
            revision,
            parent: Some(target.head),
            merge_parent: None,
            branch: target_branch.to_string(),
            timestamp: now,
            actor: meta_actor,
            message: meta_message,
            is_system: meta_is_system,
            changed_nodes: change_infos,
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation,
        };
        self.revision_repo
            .store_revision_meta(tenant_id, repo_id, rev_meta)
            .await?;

        Ok(CrossBranchCopySummary {
            copied,
            deleted,
            revision,
            changes,
        })
    }
}

/// Parent path of a node path (`"/a/b" -> "/a"`, `"/a" -> "/"`).
fn parent_path_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
    }
}
