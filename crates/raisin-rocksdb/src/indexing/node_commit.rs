//! The commit step of one repository write (plan Phase 7b): lock the nodes it
//! writes ([`super::node_lock`]), re-validate each staged index write against
//! what is stored NOW ([`StagedDeltaCheck::revalidate_final`]), write the
//! batch on the blocking pool, release.
//!
//! The transaction commit does the same three steps in its own order
//! (`transaction/commit/mod.rs`, around the branch record lock); everything
//! else that writes nodes outside a transaction collects a [`NodeCommit`] and
//! either calls [`NodeCommit::write`] or hands it to
//! `BranchRepositoryImpl::write_batch_with_head_as`.

use super::node_lock::{lock_nodes, NodeWriteGuard};
use super::StagedDeltaCheck;
use raisin_error::Result;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// The nodes one commit writes on one branch, and the index writes to
/// re-validate at its commit.
#[derive(Debug, Clone, Default)]
pub struct NodeCommit {
    tenant_id: String,
    repo_id: String,
    branch: String,
    node_ids: Vec<String>,
    /// Each with the node's FINAL state in this commit (`None`: deleted).
    checks: Vec<(StagedDeltaCheck, Option<Node>)>,
    /// UNIQUE claims the writer knows other nodes hold at the commit's
    /// revision without staging them (a merge resolution: the claims the
    /// merged branches give to other nodes, `merge::merged_view`).
    held: crate::repositories::nodes::CommitClaims,
    /// Localized name uniqueness checks, run again under the branch record
    /// lock (`localized_name::unique::deferred`). Empty unless enforced.
    names: Vec<crate::localized_name::unique::NameCheck>,
    /// Write nothing when a checked node was written since its read
    /// ([`Self::only_if_unchanged`]).
    conditional: bool,
    /// The revision record a commit that advances its branch HEAD stores
    /// ([`Self::record_revision`]).
    record: Option<raisin_storage::RevisionMeta>,
}

impl NodeCommit {
    pub fn new(tenant_id: &str, repo_id: &str, branch: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            ..Default::default()
        }
    }

    /// A node this commit writes without a baseline to re-validate (a node
    /// with a freshly minted id, a copy's; an ordering-only write). It is
    /// locked all the same: another writer's re-validation must not see it
    /// land half-way.
    pub fn touch(&mut self, node_id: &str) -> &mut Self {
        self.node_ids.push(node_id.to_string());
        self
    }

    /// A node whose staged index write is re-validated at commit, ending in
    /// `final_version` (`None` for a delete).
    pub fn check(&mut self, check: StagedDeltaCheck, final_version: Option<Node>) -> &mut Self {
        self.node_ids.push(check.node_id().to_string());
        self.checks.push((check, final_version));
        self
    }

    /// Claims other nodes hold that no correction of this commit may end
    /// (see the `held` field).
    pub(crate) fn hold_external(
        &mut self,
        claims: &crate::repositories::nodes::CommitClaims,
    ) -> &mut Self {
        self.held.extend(claims);
        self
    }

    /// A localized name uniqueness check to run again under the branch
    /// record lock (`None`, the common case: nothing is enforced).
    pub(crate) fn check_name(
        &mut self,
        check: Option<crate::localized_name::unique::NameCheck>,
    ) -> &mut Self {
        self.names.extend(check);
        self
    }

    /// Make the commit CONDITIONAL: under the node lock, when any checked
    /// node has been written since the check recorded it
    /// ([`StagedDeltaCheck::superseded`]), the commit writes nothing — the
    /// write it staged is dropped, not corrected. For a writer that rewrites
    /// the version it READ and must never put that content back over a
    /// newer one (the timestamp backfill, plan Phase 13g review). Honoured
    /// by `BranchRepositoryImpl::write_batch_with_head_as`, which reports it
    /// (`HeadWrite::Superseded`).
    pub fn only_if_unchanged(&mut self) -> &mut Self {
        self.conditional = true;
        self
    }

    /// Whether a conditional commit must write nothing (see
    /// [`Self::only_if_unchanged`]). Call with [`Self::lock`] held.
    pub(crate) fn superseded(&self, db: &DB) -> Result<bool> {
        if !self.conditional {
            return Ok(false);
        }
        for (check, _) in &self.checks {
            if check.superseded(db)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The revision record (`RevisionMeta`) to store when the commit
    /// advances its branch HEAD to `meta.revision`: its `parent` is set to
    /// the HEAD it replaces, under the branch record lock, in the commit's
    /// batch (`BranchRepositoryImpl::write_batch_with_head_as`). Without it
    /// the next commit's parent is a revision with no record, and every
    /// ancestry walk (divergence, merge, conflict detection) stops there.
    pub(crate) fn record_revision(&mut self, meta: raisin_storage::RevisionMeta) -> &mut Self {
        self.record = Some(meta);
        self
    }

    /// The revision record carried (see [`Self::record_revision`]).
    pub(crate) fn revision_record(&self) -> Option<&raisin_storage::RevisionMeta> {
        self.record.as_ref()
    }

    /// Run the carried localized name checks. Call with the branch record
    /// lock held.
    pub(crate) fn check_names(&self, db: &DB) -> Result<()> {
        for check in &self.names {
            check.run(db, &self.tenant_id, &self.repo_id, &self.branch)?;
        }
        Ok(())
    }

    /// The branch record lock, with the carried name checks passed under
    /// it, when the commit carries any (`None` otherwise: no lock taken).
    async fn lock_for_names(
        &self,
        db: &DB,
    ) -> Result<Option<tokio::sync::MutexGuard<'static, ()>>> {
        if self.names.is_empty() {
            return Ok(None);
        }
        let lock =
            crate::repositories::lock_branch_record(&self.tenant_id, &self.repo_id, &self.branch)
                .await;
        self.check_names(db)?;
        Ok(Some(lock))
    }

    /// Whether the commit writes no node at all.
    pub fn is_empty(&self) -> bool {
        self.node_ids.is_empty()
    }

    /// Lock every node of the commit (sorted; see `node_lock`).
    pub async fn lock(&self, db: &DB) -> NodeWriteGuard {
        lock_nodes(
            db,
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            self.node_ids.iter().map(String::as_str),
        )
        .await
    }

    /// Append every correction the stored versions now call for. Call with
    /// [`Self::lock`] held. Returns how many nodes were corrected.
    ///
    /// Every node's UNIQUE claims are collected FIRST: a correction is
    /// appended after the whole batch, so one that ends a value another node
    /// of this commit has just taken at the same revision (a promotion handing
    /// a value over, a prune and its replacement) would erase that node's
    /// claim (plan Phase 13a).
    pub fn revalidate(&self, db: &Arc<DB>, batch: &mut WriteBatch) -> Result<usize> {
        // Entries staged under a workspace declaration that has changed since.
        super::property_delta::fail_changed_declarations(
            db,
            self.checks.iter().map(|(check, _)| check),
        )?;
        let mut held = self.held.clone();
        for (check, final_version) in &self.checks {
            check.hold_claims(db, final_version.as_ref(), &mut held);
        }
        let mut corrected = 0;
        for (check, final_version) in &self.checks {
            if check.revalidate_final_held(db, batch, final_version.as_ref(), Some(&held))? {
                corrected += 1;
            }
        }
        Ok(corrected)
    }

    /// The whole commit step for a batch that moves no branch HEAD: lock,
    /// re-validate, write on the blocking pool, release. A commit carrying
    /// localized name checks also holds the branch record lock (after the
    /// nodes: the one lock order) from its checks through the write.
    pub async fn write(&self, db: &Arc<DB>, mut batch: WriteBatch) -> Result<()> {
        let guard = self.lock(db).await;
        self.revalidate(db, &mut batch)?;
        let branch_lock = self.lock_for_names(db).await?;
        guard.before_write().await;
        let db = db.clone();
        // The guards move INTO the blocking task: dropping this future while
        // the write is in flight must not release them (`node_lock`).
        tokio::task::spawn_blocking(move || {
            guard.in_write();
            let written = db.write(batch);
            drop(branch_lock);
            drop(guard);
            written
        })
        .await
        .map_err(|e| raisin_error::Error::storage(format!("Write task failed to join: {}", e)))?
        .map_err(|e| raisin_error::Error::storage(format!("Atomic write failed: {}", e)))?;
        Ok(())
    }
}
