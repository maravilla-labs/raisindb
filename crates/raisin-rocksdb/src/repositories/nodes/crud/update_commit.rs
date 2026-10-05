//! The commit step of `update_impl_as`: write the staged batch with its
//! branch HEAD advance and revision record, then capture it for replication.
//!
//! Every revision this funnel moves the HEAD to gets a revision record
//! (`RevisionMeta`, parent = the HEAD it replaced), stored in the same batch
//! and replicated as `CreateRevisionMeta` before the HEAD update — as the
//! transaction commit does. Without it the next commit's parent is a
//! revision with no record, and every ancestry walk (`calculate_divergence`,
//! merge, conflict detection) stops there: a merge base of `HLC(0,0)` and
//! changes silently missing from the merge (plan Phase 13g review).

use super::super::NodeRepositoryImpl;
use super::UpdateMode;
use crate::indexing::{NodeCommit, StagedDeltaCheck};
use crate::localized_name::unique::NameCheck;
use crate::repositories::nodes::WriteAttribution;
use crate::repositories::HeadWrite;
use crate::secret_store::StoredSecret;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// What one `update_impl_as` write commits.
pub(super) struct StagedUpdate<'a> {
    pub(super) tenant_id: &'a str,
    pub(super) repo_id: &'a str,
    pub(super) branch: &'a str,
    pub(super) workspace: &'a str,
    pub(super) node: &'a Node,
    pub(super) revision: HLC,
    /// The write reuses a stored version's revision (in place).
    pub(super) reused_revision: bool,
    pub(super) mode: &'a UpdateMode,
    pub(super) attribution: WriteAttribution<'a>,
    pub(super) staged_check: StagedDeltaCheck,
    pub(super) name_check: Option<NameCheck>,
    pub(super) vault_actor: &'a str,
    pub(super) secret_ops: &'a [StoredSecret],
}

impl NodeRepositoryImpl {
    /// Commit `batch` (see the module doc). Returns `false` when a
    /// conditional (backfill) commit found the node written since its read
    /// and wrote nothing.
    pub(super) async fn commit_update(
        &self,
        batch: WriteBatch,
        staged: StagedUpdate<'_>,
    ) -> Result<bool> {
        let StagedUpdate {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            revision,
            reused_revision,
            mode,
            attribution,
            staged_check,
            name_check,
            vault_actor,
            secret_ops,
        } = staged;
        // Branch HEAD advance rides in the same batch; the write happens under
        // the branch record lock so a concurrent writer cannot regress HEAD.
        let mut commit = NodeCommit::new(tenant_id, repo_id, branch);
        commit.check(staged_check, Some(node.clone()));
        commit.check_name(name_check);
        if !mode.is_edit() {
            // A backfill never puts the content it read back over a newer
            // version (`update_mode`'s module doc).
            commit.only_if_unchanged();
        }
        if !reused_revision {
            commit.record_revision(mode.revision_record(
                branch,
                workspace,
                node,
                revision,
                attribution.actor,
            ));
        }
        let (updated_branch, revision_meta) = match self
            .branch_repo
            .write_batch_with_head_as(
                batch,
                tenant_id,
                repo_id,
                branch,
                revision,
                reused_revision,
                Some(&commit),
            )
            .await?
        {
            HeadWrite::Written { branch, meta } => (branch, meta),
            HeadWrite::Superseded => {
                tracing::debug!(
                    node_id = %node.id,
                    branch,
                    "timestamp backfill skipped a node written since its read"
                );
                return Ok(false);
            }
        };

        // The revision record BEFORE the HEAD update that names it (the
        // transaction commit's order).
        if let Some(meta) = revision_meta {
            self.capture_revision_meta(tenant_id, repo_id, *meta).await;
        }
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                branch,
                &updated_branch,
                revision,
            )
            .await;

        // Secret versions BEFORE the node: same `(tenant, repo)` lane as the
        // node operation below, and earlier in it — which is what makes a
        // peer's causal buffer hold the node snapshot until the secret has
        // landed. See `replication/operation_capture/secret_ops.rs`.
        self.capture_secret_versions(tenant_id, repo_id, branch, vault_actor, secret_ops)
            .await;

        // Full-snapshot ApplyRevision (like the transaction commit path).
        // Per-property SetProperty ops cannot express removed properties or
        // path/name/type changes, so peers would drift.
        self.capture_apply_revision_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            vec![(
                node.clone(),
                raisin_replication::operation::ReplicatedNodeChangeKind::Upsert,
            )],
            revision,
            attribution,
        )
        .await;
        Ok(true)
    }
}
