//! Commit-time re-validation of a staged property-index write — and of the
//! compound write staged against the same baseline (plan Phases 7, 8, 7b).
//!
//! A baseline is proven when the batch is STAGED, not when it commits. Two
//! writers of one node can both read the same stored predecessor P and both
//! stage against it: A commits at R1 and tombstones `title = 'x'`, then B
//! commits at R2 > R1 having skipped `title = 'x'` as unchanged against P —
//! the newest version (B's) has `title = 'x'` and the index has a tombstone
//! at R1 above P's entry, so `title = 'x'` never matches again. The mirror
//! case is a commit landing BELOW a version committed meanwhile with skips
//! (out of revision order): its tombstones at R mask the successor's
//! unchanged entries, which sit below R.
//!
//! The flag does not matter: with skipping OFF a write still derives the
//! entries it ENDS from the version it read, so two writers of one node that
//! both read P each end P's values and neither ends the other's — the loser's
//! new value stays live forever.
//!
//! So every write records what it assumed BEFORE its baseline is read
//! ([`StagedDeltaCheck::before_read`] / [`StagedDeltaCheck::capture`]): the
//! node's newest stored NODES version (revision AND a hash of its bytes, so an
//! in-place rewrite at the same revision counts as a change) and its newest
//! NODE_PATH entry. A writer that reads many nodes in bulk (a move, a cascade
//! delete, a merge, a promotion) records [`StagedDeltaCheck::always`] instead:
//! nothing to compare, the commit always re-derives.
//!
//! The commit re-reads that under the node's commit lock
//! (`indexing::node_lock`, plan Phase 7b — no other writer of the node can
//! land until this commit has written) and, when it changed, appends a
//! corrective write of the node's FINAL version against what is stored NOW
//! ([`StagedDeltaCheck::revalidate_final`]): for a version, every entry put
//! at R, the true predecessor's values tombstoned and every successor's
//! entries re-asserted at its own revision; for a delete, the true
//! predecessor's entries tombstoned at R and the successors re-asserted. Puts
//! in a `WriteBatch` apply in order, so the correction overrides any staged
//! write of the same key. The correction IS the one writer
//! ([`write_property_index_delta`] with `Full` / `OutOfOrder`,
//! [`tombstone_all_entries`] + [`reassert_successors`]) — there is no second
//! derivation.
//!
//! A NODE_PATH entry with no NODES version at its revision means an ancestor
//! move re-keyed this node's `__parent_path` compound entries WITHOUT a
//! record write. That cannot be corrected from NODES (the entries it wrote
//! were derived from the version it read), so it fails the workspace's
//! compound indexes CLOSED instead: marked `NotBuilt`, build requested — the
//! cold-definition answer. An ordinary record write changes NODE_PATH too
//! (every record writer writes both), and that is corrected from NODES like
//! any racing version (`staged_markers`).
//!
//! An IN-PLACE (`versionable=false`) write records its check too: a racing
//! in-place rewrite at the same revision changes the NODES marker's hash, and
//! the correction then diffs against the content stored AT the revision now
//! (the racer's), placed exactly like the staged write. A CREATE records one
//! (markers usually `(None, None)`): a version of the same id committed in
//! between is ended like any predecessor. A move's re-key of a descendant it
//! does not rewrite is checked against the stored version
//! ([`StagedDeltaCheck::rekey`], `staged_compound::correct_rekey`).
//!
//! Recording BEFORE the baseline read makes the race safe in the one
//! direction that matters: a commit landing between the two reads makes the
//! record older than the baseline, which only costs an unneeded correction.

use super::staged_markers::{index_only_path_write, markers, Markers};
use crate::indexing::IndexCtx;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::sync::atomic::{AtomicU64, Ordering};

static CORRECTED: AtomicU64 = AtomicU64::new(0);

/// Staged property-index writes corrected at commit since process start
/// (the stored versions changed between staging and commit, or the writer
/// asked for an unconditional re-derivation).
pub fn corrected_staged_writes() -> u64 {
    CORRECTED.load(Ordering::Relaxed)
}

/// What a staged write of one node assumed about its stored versions.
#[derive(Debug, Clone)]
pub struct StagedDeltaCheck {
    scope: Scope,
    revision: HLC,
    mode: Mode,
    /// The workspace-declaration change sequence when the check was recorded
    /// (`staged_declarations`): a declaration that changed after it makes the
    /// staged workspace-index entries suspect.
    declarations_seq: u64,
}

#[derive(Debug, Clone)]
enum Mode {
    /// The markers recorded before the baseline read. `in_place`: the write
    /// reuses its revision (`Some(resolved_targets)`, whether the staged
    /// write placed entries through `InPlaceTargets`).
    Recorded {
        markers: Markers,
        in_place: Option<bool>,
    },
    /// Nothing recorded: re-derive at commit whatever is stored.
    Always,
    /// A compound re-key (a move) of a node the commit does not rewrite,
    /// derived from the version the move LISTED.
    Rekey(Box<Node>),
}

/// [`StagedDeltaCheck`] before the write's revision is known.
#[derive(Debug, Clone)]
pub struct PendingDeltaCheck {
    scope: Scope,
    markers: Markers,
    declarations_seq: u64,
}

#[derive(Debug, Clone)]
struct Scope {
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    node_id: String,
}

impl Scope {
    fn new(ctx: &IndexCtx<'_>, node_id: &str) -> Self {
        Self {
            tenant_id: ctx.tenant_id.to_string(),
            repo_id: ctx.repo_id.to_string(),
            branch: ctx.branch.to_string(),
            workspace: ctx.workspace.to_string(),
            node_id: node_id.to_string(),
        }
    }

    fn ctx(&self) -> IndexCtx<'_> {
        IndexCtx::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &self.workspace,
        )
    }
}

impl PendingDeltaCheck {
    /// The check, once the write's (fresh) revision is known.
    pub fn at(self, revision: &HLC) -> StagedDeltaCheck {
        self.with(revision, None)
    }

    /// The check of an IN-PLACE write at the reused `revision`;
    /// `resolved_targets` says whether the staged write placed its entries
    /// through `InPlaceTargets` (the correction places them the same way).
    pub fn in_place_at(self, revision: &HLC, resolved_targets: bool) -> StagedDeltaCheck {
        self.with(revision, Some(resolved_targets))
    }

    /// The revisions of the newest NODES version and the newest NODE_PATH
    /// entry recorded (`None`: none stored) — what a write that rewrites the
    /// version it read IN PLACE checks it may (the timestamp backfill, plan
    /// Phase 13g review).
    pub fn recorded_revisions(&self) -> (Option<HLC>, Option<HLC>) {
        (
            self.markers.0.map(|(revision, _)| revision),
            self.markers.1.map(|(revision, _)| revision),
        )
    }

    fn with(self, revision: &HLC, in_place: Option<bool>) -> StagedDeltaCheck {
        StagedDeltaCheck {
            scope: self.scope,
            revision: *revision,
            mode: Mode::Recorded {
                markers: self.markers,
                in_place,
            },
            declarations_seq: self.declarations_seq,
        }
    }
}

impl StagedDeltaCheck {
    /// Record the node's newest stored versions, BEFORE the write reads the
    /// version it derives its index delta from (see the module doc).
    pub fn before_read(db: &DB, ctx: &IndexCtx<'_>, node_id: &str) -> Result<PendingDeltaCheck> {
        Ok(PendingDeltaCheck {
            scope: Scope::new(ctx, node_id),
            // Before the markers, and so before the write reads anything.
            declarations_seq: crate::indexing::compound::workspace_defs::change_seq(),
            markers: markers(db, ctx, node_id)?,
        })
    }

    /// [`Self::before_read`] when the revision is already known.
    pub fn capture(db: &DB, ctx: &IndexCtx<'_>, node_id: &str, revision: &HLC) -> Result<Self> {
        Ok(Self::before_read(db, ctx, node_id)?.at(revision))
    }

    /// A check with nothing recorded: the commit always re-derives the node's
    /// index write against what is stored (bulk writers that read many nodes
    /// before staging: moves, re-stamps, cascade deletes, merges, promotion).
    pub fn always(ctx: &IndexCtx<'_>, node_id: &str, revision: &HLC) -> Self {
        Self {
            scope: Scope::new(ctx, node_id),
            revision: *revision,
            mode: Mode::Always,
            declarations_seq: crate::indexing::compound::workspace_defs::change_seq(),
        }
    }

    /// A move re-keyed `listed`'s compound entries at `revision` (to its moved
    /// path) WITHOUT rewriting its record. The commit compares `listed` with
    /// the version stored now and re-derives the re-key from the stored one
    /// when an update committed between the listing and the move's commit.
    pub fn rekey(ctx: &IndexCtx<'_>, listed: &Node, revision: &HLC) -> Self {
        Self {
            scope: Scope::new(ctx, &listed.id),
            revision: *revision,
            mode: Mode::Rekey(Box::new(listed.clone())),
            declarations_seq: crate::indexing::compound::workspace_defs::change_seq(),
        }
    }

    /// Whether a version of the node was written since the markers were
    /// recorded — a record write (the NODES marker changed: revision or, for
    /// an in-place rewrite, bytes) or an index-only re-key (an ancestor
    /// move). A NODE_PATH entry at a revision NODES already held (the
    /// `node_path` backfill) is not one. A check that recorded nothing is
    /// never superseded. Call under the node's commit lock: a CONDITIONAL
    /// commit (`NodeCommit::only_if_unchanged`) then writes nothing.
    pub fn superseded(&self, db: &DB) -> Result<bool> {
        let Mode::Recorded {
            markers: at_stage, ..
        } = &self.mode
        else {
            return Ok(false);
        };
        let ctx = self.scope.ctx();
        let now = markers(db, &ctx, &self.scope.node_id)?;
        Ok(now.0 != at_stage.0
            || (now.1 != at_stage.1
                && index_only_path_write(db, &ctx, &self.scope.node_id, &at_stage.1)?))
    }

    pub fn workspace(&self) -> &str {
        &self.scope.workspace
    }

    pub fn node_id(&self) -> &str {
        &self.scope.node_id
    }

    /// `(tenant, repo, branch, workspace)` and the declaration sequence the
    /// check was recorded at (`staged_declarations`).
    pub(super) fn declarations_scope(&self) -> ((&str, &str, &str, &str), u64) {
        (
            (
                &self.scope.tenant_id,
                &self.scope.repo_id,
                &self.scope.branch,
                &self.scope.workspace,
            ),
            self.declarations_seq,
        )
    }

    /// [`Self::revalidate_final`] for a write that stores `node` at the
    /// staged revision.
    pub fn revalidate(
        &self,
        db: &std::sync::Arc<DB>,
        batch: &mut WriteBatch,
        node: &Node,
    ) -> Result<bool> {
        self.revalidate_final(db, batch, Some(node))
    }

    /// Under the node's commit lock: when the node's stored versions changed
    /// since the write recorded them (or it recorded nothing), append to
    /// `batch` the write of `final_version` — the version this commit stores
    /// at the staged revision (for a re-key: the moved record), `None` for a
    /// delete — against what is stored now. Returns whether a correction was
    /// written.
    pub fn revalidate_final(
        &self,
        db: &std::sync::Arc<DB>,
        batch: &mut WriteBatch,
        final_version: Option<&Node>,
    ) -> Result<bool> {
        self.revalidate_final_held(db, batch, final_version, None)
    }

    /// Record in `held` the UNIQUE claims `final_version` holds at this
    /// check's revision (from the definitions cache; a cold type records
    /// none). A commit collects them for every node it writes BEFORE any
    /// correction, so that a correction never ends a claim another node of
    /// the same batch has just staged (plan Phase 13a).
    pub(crate) fn hold_claims(
        &self,
        db: &DB,
        final_version: Option<&Node>,
        held: &mut crate::repositories::nodes::CommitClaims,
    ) {
        if let Some(node) = final_version {
            super::staged_neighbours::hold_claims(
                db,
                &self.scope.ctx(),
                node,
                &self.revision,
                held,
            );
        }
    }

    /// [`Self::revalidate_final`] within a commit whose other nodes' claims
    /// are `held` (see [`Self::hold_claims`]).
    pub(crate) fn revalidate_final_held(
        &self,
        db: &std::sync::Arc<DB>,
        batch: &mut WriteBatch,
        final_version: Option<&Node>,
        held: Option<&crate::repositories::nodes::CommitClaims>,
    ) -> Result<bool> {
        let ctx = self.scope.ctx();
        let node_id = &self.scope.node_id;
        let in_place = match &self.mode {
            Mode::Rekey(listed) => {
                let corrected = super::staged_compound::correct_rekey(
                    db,
                    batch,
                    &ctx,
                    listed,
                    final_version,
                    &self.revision,
                )?;
                if corrected {
                    CORRECTED.fetch_add(1, Ordering::Relaxed);
                }
                return Ok(corrected);
            }
            Mode::Always => None,
            Mode::Recorded {
                markers: at_stage,
                in_place,
            } => {
                let now = markers(db, &ctx, node_id)?;
                if now == *at_stage {
                    return Ok(false);
                }
                if now.1 != at_stage.1 && index_only_path_write(db, &ctx, node_id, &at_stage.1)? {
                    // An index-only write (an ancestor move) landed meanwhile.
                    let types: Vec<&str> = final_version
                        .map(|n| n.node_type.as_str())
                        .into_iter()
                        .collect();
                    super::staged_compound::fail_compound_closed(
                        db,
                        &ctx,
                        &types,
                        crate::compound_state::StaleScope::All,
                    )?;
                    if now.0 == at_stage.0 {
                        return Ok(true);
                    }
                } else if now.0 == at_stage.0 {
                    // NODE_PATH entries at revisions NODES already held (the
                    // `node_path` backfill): no version changed.
                    return Ok(false);
                }
                *in_place
            }
        };
        super::staged_rederive::rederive(
            db,
            batch,
            &ctx,
            node_id,
            &self.revision,
            in_place,
            final_version,
            held,
        )?;
        CORRECTED.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            node_id = %self.scope.node_id,
            revision = %self.revision,
            "property index re-derived at commit against the stored versions"
        );
        Ok(true)
    }
}
