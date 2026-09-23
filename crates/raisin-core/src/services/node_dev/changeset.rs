// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Changesets: dry-run, propose (a reviewable record), commit, discard, and
//! one-shot apply.
//!
//! The commit protocol, inside the branch's exclusive section:
//!
//! 1. load the record — a committed record answers with its stored receipt
//!    (`replayed: true`), which is what makes an idempotency key safe to
//!    retry after a crash, a timeout or a redelivered continuation;
//! 2. re-plan against the world as it is NOW, with the committing caller's
//!    rights — a stale revision, a vanished target or an occupied destination
//!    is returned as a conflict and nothing is written;
//! 3. when the caller binds the commit to a reviewed digest, a different
//!    digest is a conflict too (the approval was for another change);
//! 4. apply every op AND the committed record in ONE transaction;
//! 5. read back the exact receipt and store it on the record.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage};

use super::changeset_apply::skeleton;
use super::changeset_store::{changeset_id, RECORD_WORKSPACE};
use super::changeset_types::*;
use super::types::*;
use super::{DevScope, NodeDevService};

/// Who is calling, as a stable string.
pub fn owner_of(auth: &AuthContext) -> String {
    auth.principal_id().unwrap_or_else(|| auth.actor_id())
}

/// System or system-admin.
pub fn is_admin(auth: &AuthContext) -> bool {
    auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin)
}

fn branch_key(scope: &DevScope) -> String {
    format!(
        "node_dev_commit:{}/{}/{}",
        scope.tenant, scope.repo, scope.branch
    )
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    /// Plan without storing anything.
    pub async fn dry_run(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        req: &ChangesetRequest,
    ) -> DevResult<ChangesetPlan> {
        self.plan(scope, auth, req).await
    }

    /// Store a reviewable changeset. Proposing the same idempotency key with
    /// the same ops returns the existing record; with other ops it is refused.
    pub async fn propose(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        req: ChangesetRequest,
    ) -> DevResult<(ChangesetRecord, bool)> {
        let owner = owner_of(auth);
        let id = changeset_id(scope, &owner, req.idempotency_key.as_deref());
        if req.idempotency_key.is_some() {
            if let Some(existing) = self.load_record(scope, &id).await? {
                if existing.request.ops != req.ops {
                    return Err(NodeDevError::new(
                        409,
                        "idempotency_key_reused",
                        "this idempotency key already names a different changeset",
                    ));
                }
                return Ok((existing, false));
            }
        }
        let plan = self.plan(scope, auth, &req).await?;
        let rec = ChangesetRecord {
            changeset_id: id,
            idempotency_key: req.idempotency_key.clone(),
            owner,
            repository: scope.repo.clone(),
            branch: scope.branch.clone(),
            request: req,
            status: ChangesetStatus::Proposed,
            plan,
            created_at: chrono::Utc::now().to_rfc3339(),
            committed_by: None,
            receipt: None,
        };
        self.save_record(scope, auth, &rec).await?;
        Ok((rec, true))
    }

    /// Read a changeset (its owner, or an administrator).
    pub async fn get_changeset(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        id: &str,
    ) -> DevResult<ChangesetRecord> {
        let rec = self
            .load_record(scope, id)
            .await?
            .ok_or_else(|| NodeDevError::not_found(format!("changeset {id}")))?;
        if rec.owner != owner_of(auth) && !is_admin(auth) {
            // A reviewer who may perform every op may read it too.
            self.plan(scope, auth, &rec.request)
                .await
                .map_err(|_| NodeDevError::not_found(format!("changeset {id}")))?;
        }
        Ok(rec)
    }

    /// List the caller's changesets on this branch (all, for an administrator).
    pub async fn list_changesets(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        status: Option<ChangesetStatus>,
        limit: usize,
    ) -> DevResult<Vec<ChangesetRecord>> {
        let nodes = self
            .storage
            .nodes()
            .deep_children_flat(scope.storage(RECORD_WORKSPACE), "/changesets", 2, None)
            .await?;
        let owner = owner_of(auth);
        let admin = is_admin(auth);
        let mut out: Vec<ChangesetRecord> = nodes
            .iter()
            .filter_map(|n| match n.properties.get("record_json") {
                Some(PropertyValue::String(s)) => serde_json::from_str(s).ok(),
                _ => None,
            })
            .filter(|r: &ChangesetRecord| admin || r.owner == owner)
            .filter(|r| status.is_none_or(|s| r.status == s))
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit);
        Ok(out)
    }

    /// Abandon a proposed changeset.
    pub async fn discard(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        id: &str,
    ) -> DevResult<ChangesetRecord> {
        let _section = self.sections.enter(&branch_key(scope)).await?;
        let mut rec = self.get_changeset(scope, auth, id).await?;
        match rec.status {
            ChangesetStatus::Committed => Err(NodeDevError::new(
                409,
                "already_committed",
                "a committed changeset cannot be discarded; apply its inverse",
            )),
            ChangesetStatus::Discarded => Ok(rec),
            ChangesetStatus::Proposed => {
                rec.status = ChangesetStatus::Discarded;
                self.save_record(scope, auth, &rec).await?;
                Ok(rec)
            }
        }
    }

    /// Commit a proposed changeset. `expected_digest` binds the commit to
    /// the digest a reviewer approved.
    pub async fn commit(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        id: &str,
        expected_digest: Option<&str>,
    ) -> DevResult<CommitOutcome> {
        let _section = self.sections.enter(&branch_key(scope)).await?;
        let rec = self
            .load_record(scope, id)
            .await?
            .ok_or_else(|| NodeDevError::not_found(format!("changeset {id}")))?;
        self.commit_locked(scope, auth, rec, expected_digest, None)
            .await
    }

    /// Propose and commit in one call. With an idempotency key, a replay
    /// returns the first commit's receipt and writes nothing.
    pub async fn apply(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        req: ChangesetRequest,
    ) -> DevResult<CommitOutcome> {
        let _section = self.sections.enter(&branch_key(scope)).await?;
        let owner = owner_of(auth);
        let id = changeset_id(scope, &owner, req.idempotency_key.as_deref());
        if req.idempotency_key.is_some() {
            if let Some(existing) = self.load_record(scope, &id).await? {
                if existing.request.ops != req.ops {
                    return Err(NodeDevError::new(
                        409,
                        "idempotency_key_reused",
                        "this idempotency key already names a different changeset",
                    ));
                }
                return self.commit_locked(scope, auth, existing, None, None).await;
            }
        }
        // One-shot: the record is written only WITH the changes (one commit),
        // never as a separate proposal first.
        let plan = self.plan(scope, auth, &req).await?;
        let rec = ChangesetRecord {
            changeset_id: id,
            idempotency_key: req.idempotency_key.clone(),
            owner,
            repository: scope.repo.clone(),
            branch: scope.branch.clone(),
            request: req,
            status: ChangesetStatus::Proposed,
            plan: plan.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            committed_by: None,
            receipt: None,
        };
        self.commit_locked(scope, auth, rec, None, Some(plan)).await
    }

    async fn commit_locked(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        mut rec: ChangesetRecord,
        expected_digest: Option<&str>,
        fresh: Option<ChangesetPlan>,
    ) -> DevResult<CommitOutcome> {
        match rec.status {
            ChangesetStatus::Committed => {
                let mut receipt = rec.receipt.clone().ok_or_else(|| {
                    NodeDevError::new(500, "corrupt_record", "committed changeset has no receipt")
                })?;
                receipt.replayed = true;
                return Ok(CommitOutcome::Committed { receipt });
            }
            ChangesetStatus::Discarded => {
                return Err(NodeDevError::new(
                    409,
                    "discarded",
                    "the changeset was discarded",
                ));
            }
            ChangesetStatus::Proposed => {}
        }
        let plan = match fresh {
            Some(p) => p,
            None => self.plan(scope, auth, &rec.request).await?,
        };
        let mut conflicts = plan.conflicts.clone();
        if let Some(d) = expected_digest {
            if d != plan.digest {
                conflicts.push(Conflict {
                    index: 0,
                    code: "digest_mismatch".into(),
                    message: "the changeset no longer matches the reviewed digest".into(),
                    expected: Some(d.to_string()),
                    actual: None,
                });
            }
        }
        if !conflicts.is_empty() {
            return Ok(CommitOutcome::Conflict {
                changeset_id: rec.changeset_id,
                conflicts,
                digest: plan.digest,
            });
        }
        rec.status = ChangesetStatus::Committed;
        rec.committed_by = Some(owner_of(auth));
        rec.receipt = Some(Receipt {
            changeset_id: rec.changeset_id.clone(),
            repository: scope.repo.clone(),
            branch: scope.branch.clone(),
            committed_revision: None,
            ops: plan.ops.iter().map(|p| skeleton(scope, p)).collect(),
            replayed: false,
        });
        rec.plan = plan;
        let copies = self
            .apply_plan(scope, auth, &rec.request, &rec.plan, &rec)
            .await?;
        let receipt = self
            .build_receipt(scope, &rec.changeset_id, &rec.plan, &copies)
            .await?;
        rec.receipt = Some(receipt.clone());
        // Best effort: the commit and its skeleton receipt are already
        // durable; this only adds the post-commit revisions.
        if let Err(e) = self.save_record(scope, auth, &rec).await {
            tracing::warn!(changeset = %rec.changeset_id, error = %e, "receipt stamp failed");
        }
        Ok(CommitOutcome::Committed { receipt })
    }
}
