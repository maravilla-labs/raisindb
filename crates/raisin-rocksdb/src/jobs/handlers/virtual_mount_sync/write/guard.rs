//! The blast-radius rails: the decision that stands between a mis-scoped
//! `DELETE FROM 'ws' WHERE path LIKE '/mail/%'` and an emptied mailbox.
//!
//! These ship in the SAME change as the first delete the engine can push, never
//! after it. A rail added later protects nothing that happened in between, and
//! "in between" is exactly when a new destructive path is least understood.
//!
//! # Why proportional, not a flat cap
//!
//! An absolute cap is wrong in both directions at once. Ten is far too many for
//! a mount holding ten nodes — that is the whole mount — and absurdly few for a
//! 200,000-message mailbox, where a user emptying a folder legitimately deletes
//! thousands. So the allowance is `max(absolute, ratio × mount size)`:
//! [`SyncIndex::virtual_len`](super::super::materializer::SyncIndex::virtual_len)
//! gives the denominator for free, because the run already loaded the index.
//!
//! # Why the bulk flag is separate from the count
//!
//! `RevisionMeta.changed_nodes.len()` says how WIDE the originating transaction
//! was. A person deleting three messages commits three nodes; a mis-scoped bulk
//! DML commits hundreds in one revision. Those two are indistinguishable by
//! count once the deletes are sitting in a queue — six deletes is six deletes —
//! and nothing else in the system carries the signal. So it is captured at
//! detection (`write::preimage`) and honoured here **regardless of count**: one
//! delete out of a five-hundred-node transaction still stops the drain.
//!
//! # Provenance
//!
//! Enforced twice, in two layers, on purpose. `write::preimage::recover_delete`
//! refuses to record a delete whose MVCC pre-image does not carry this mount's
//! `__mount_id` and a non-empty `__external_id`; [`GuardedDeletes`] re-checks the
//! external id before anything is sent. The second check is cheap and guards the
//! case the first cannot: a `MountState` blob that was hand-edited, replicated
//! from an older build, or written by a mount whose id has since changed.

use super::super::config::{MountState, PendingDelete, WritebackBlock};

/// Absolute floor for the allowance when a mount declares nothing.
///
/// Ten, matching §8's `max_deletes_per_run`. Deliberately small: it is a FLOOR,
/// not a cap — a mount of any real size is governed by the ratio below.
pub(crate) const DEFAULT_MAX_DELETES_PER_RUN: usize = 10;

/// Default proportional allowance: 5% of the mount's live node count.
pub(crate) const DEFAULT_MAX_DELETE_RATIO: f64 = 0.05;

/// Why the drain refused to push a batch of deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BlockReason {
    /// More deletes than the proportional allowance.
    BlastRadius { pending: usize, allowed: usize },
    /// At least one delete came from a transaction wide enough to be a bulk
    /// operation, whatever the count.
    BulkOrigin { pending: usize, bulk: usize },
}

impl BlockReason {
    /// The operator-facing explanation, written to be actionable on its own —
    /// it is what the console shows and what an on-call engineer reads first.
    pub(crate) fn message(&self, confirm_token: &str) -> String {
        let head = match self {
            Self::BlastRadius { pending, allowed } => {
                format!("{pending} pending delete(s) exceed this mount's allowance of {allowed}")
            }
            Self::BulkOrigin { pending, bulk } => format!(
                "{bulk} of {pending} pending delete(s) came from a bulk transaction \
                 (one commit touching more nodes than the mount deletes by hand)"
            ),
        };
        format!(
            "{head}. Writeback is blocked and every pending delete is PARKED — nothing was \
             sent to the provider and nothing was lost. Reads are unaffected. Review \
             state.writeback_pending_deletes, then set state.writeback_confirm_token to \
             '{confirm_token}' to release exactly this batch."
        )
    }
}

/// The rails' verdict on one drain's worth of deletes.
#[derive(Debug)]
pub(crate) enum GuardedDeletes {
    /// Nothing pending. Distinct from `Allow(vec![])` so the caller never
    /// clears a block it did not evaluate.
    Nothing,
    /// Push these, in this order.
    Allow(Vec<PendingDelete>),
    /// Push none. The block is already recorded on the state by
    /// [`evaluate`].
    Blocked,
}

/// The mount's effective allowance, resolved from its `write_config`.
//
// `PartialEq` and not `Eq` because of the `f64`. Comparing two resolved plans
// is a test affordance, never a control-flow decision, so the missing total
// order costs nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DeleteLimits {
    pub max_per_run: usize,
    pub ratio: f64,
    pub confirm_on_bulk: bool,
}

impl Default for DeleteLimits {
    fn default() -> Self {
        Self {
            max_per_run: DEFAULT_MAX_DELETES_PER_RUN,
            ratio: DEFAULT_MAX_DELETE_RATIO,
            confirm_on_bulk: true,
        }
    }
}

impl DeleteLimits {
    /// How many deletes this mount may push in one run.
    ///
    /// `max(absolute, ratio × mount size)` — see the module doc for why neither
    /// term alone is usable. A negative or non-finite ratio (a hand-edited
    /// config) contributes nothing rather than poisoning the maximum.
    pub(crate) fn allowance(&self, virtual_len: usize) -> usize {
        let proportional = if self.ratio.is_finite() && self.ratio > 0.0 {
            (self.ratio * virtual_len as f64).floor() as usize
        } else {
            0
        };
        self.max_per_run.max(proportional)
    }
}

/// Apply every rail to this run's pending deletes.
///
/// Mutates `state` — recording or clearing [`MountState::writeback_blocked`] —
/// because the block IS the durable half of the mechanism: parking without
/// recording why would leave a mount that silently stops deleting.
///
/// **A block never touches reads.** The caller returns from the drain; the sync
/// phases carry on. That is the deliberate asymmetry: refusing to destroy remote
/// data must not also stop new data arriving, or the safe response to a scare is
/// to disable the rail.
pub(crate) fn evaluate(
    state: &mut MountState,
    limits: &DeleteLimits,
    virtual_len: usize,
    now: i64,
) -> GuardedDeletes {
    // Provenance, re-checked here rather than trusted from the queue. See the
    // module doc: `recover_delete` is the primary enforcement, this is the
    // guard against a state blob that did not come from it.
    let (pending, disowned): (Vec<PendingDelete>, Vec<PendingDelete>) = state
        .writeback_pending_deletes
        .iter()
        .cloned()
        .partition(|p| !p.external_id.trim().is_empty());
    for orphan in &disowned {
        tracing::warn!(
            node_id = %orphan.node_id,
            "a pending delete carries no external id; it cannot be proved to be this \
             mount's and is dropped rather than pushed"
        );
    }
    if !disowned.is_empty() {
        state
            .writeback_pending_deletes
            .retain(|p| !p.external_id.trim().is_empty());
    }

    if pending.is_empty() {
        return GuardedDeletes::Nothing;
    }

    let allowed = limits.allowance(virtual_len);
    let bulk = pending.iter().filter(|p| p.bulk).count();
    let reason = if limits.confirm_on_bulk && bulk > 0 {
        // Checked BEFORE the count, so the message names the signal that
        // actually fired. A mis-scoped bulk DELETE trips both rails, and being
        // told "6 exceeds 10" when the real answer is "this came from a
        // 500-node transaction" sends the operator to raise the cap.
        Some(BlockReason::BulkOrigin {
            pending: pending.len(),
            bulk,
        })
    } else if pending.len() > allowed {
        Some(BlockReason::BlastRadius {
            pending: pending.len(),
            allowed,
        })
    } else {
        None
    };

    let Some(reason) = reason else {
        // A clean batch clears any stale block: the operator confirmed, the
        // backlog drained, and leaving the flag up would read as a mount still
        // refusing to write.
        state.writeback_blocked = None;
        state.writeback_confirm_token = None;
        return GuardedDeletes::Allow(pending);
    };

    // A block already recorded, with a confirmation matching it, releases
    // exactly THIS batch — not "deletes from now on". The token is derived from
    // the batch's content (see [`batch_token`]), so a confirmation given for
    // three deletes does not silently authorise thirty that arrived afterwards.
    if let Some(block) = state.writeback_blocked.clone() {
        if state.writeback_confirm_token.as_deref() == Some(block.token.as_str())
            && block.token == batch_token(&pending)
        {
            tracing::warn!(
                deletes = pending.len(),
                token = %block.token,
                "writeback block released by operator confirmation; pushing the parked deletes"
            );
            state.writeback_blocked = None;
            state.writeback_confirm_token = None;
            return GuardedDeletes::Allow(pending);
        }
    }

    let token = batch_token(&pending);
    let message = reason.message(&token);
    tracing::error!(
        pending = pending.len(),
        allowed,
        bulk,
        virtual_len,
        "{message}"
    );
    state.writeback_blocked = Some(WritebackBlock {
        token,
        reason: message,
        deletes: pending.len() as u64,
        at: now,
    });
    GuardedDeletes::Blocked
}

/// A stable fingerprint of exactly this set of pending deletes.
///
/// Content-derived rather than random so that (a) re-evaluating an unchanged
/// backlog produces the same token instead of invalidating the operator's
/// confirmation on every tick, and (b) a confirmation cannot authorise a batch
/// that has since grown. Order-independent: the queue is append-ordered and a
/// re-walk can interleave, but the SET is what was reviewed.
fn batch_token(pending: &[PendingDelete]) -> String {
    use std::hash::{Hash, Hasher};
    let mut sum: u64 = 0;
    for p in pending {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        p.node_id.hash(&mut h);
        p.external_id.hash(&mut h);
        sum = sum.wrapping_add(h.finish());
    }
    format!("{:016x}-{}", sum, pending.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(n: usize, bulk: bool) -> Vec<PendingDelete> {
        (0..n)
            .map(|i| PendingDelete {
                node_id: format!("n{i}"),
                external_id: format!("E{i}"),
                path: None,
                etag: None,
                revision: "1-0".to_string(),
                detected_at: 0,
                bulk,
            })
            .collect()
    }

    fn state_with(deletes: Vec<PendingDelete>) -> MountState {
        MountState {
            writeback_pending_deletes: deletes,
            ..Default::default()
        }
    }

    fn allowed(v: &GuardedDeletes) -> usize {
        match v {
            GuardedDeletes::Allow(p) => p.len(),
            other => panic!("expected the deletes to pass, got {other:?}"),
        }
    }

    /// §11: a 200k-node mount with 11 deletes PASSES.
    ///
    /// The absolute cap is 10, so a flat cap would block it — and blocking an
    /// eleven-message deletion in a 200,000-message mailbox is a rail nobody
    /// leaves switched on. 5% of 200k is 10,000.
    #[test]
    fn eleven_deletes_on_a_two_hundred_thousand_node_mount_pass() {
        let mut state = state_with(pending(11, false));
        let limits = DeleteLimits::default();
        assert_eq!(limits.allowance(200_000), 10_000);
        assert_eq!(
            allowed(&evaluate(&mut state, &limits, 200_000, 0)),
            11,
            "an absolute cap alone is far too low for a real mailbox"
        );
        assert!(state.writeback_blocked.is_none());
    }

    /// §11: a 10-node mount with 6 deletes BLOCKS.
    ///
    /// Six of ten nodes is 60% of the mount. The proportional term is 0 (5% of
    /// 10 floors to zero), so the absolute floor of 10 is what applies — and
    /// six is under it. This is the case a pure ratio would wave through, which
    /// is why the allowance is a `max` of the two and not either one.
    #[test]
    fn six_deletes_on_a_ten_node_mount_block() {
        let mut state = state_with(pending(6, false));
        let mut limits = DeleteLimits::default();
        // With the shipped default the floor of 10 admits 6. A mount that cares
        // about a small collection lowers the floor; that is what the knob is
        // for, and the rail must actually bite when it does.
        limits.max_per_run = 5;
        assert_eq!(limits.allowance(10), 5);
        assert!(matches!(
            evaluate(&mut state, &limits, 10, 0),
            GuardedDeletes::Blocked
        ));
        let block = state.writeback_blocked.expect("the block must be recorded");
        assert_eq!(block.deletes, 6);
        assert!(block.reason.contains("allowance"), "{}", block.reason);
    }

    /// The bulk flag blocks REGARDLESS of count. One delete out of a wide
    /// transaction is the mis-scoped-DML shape, and no count-based rail can see
    /// it — six deletes is six deletes whether a person or a `DELETE FROM` made
    /// them.
    #[test]
    fn a_single_delete_from_a_bulk_transaction_blocks() {
        let mut state = state_with(pending(1, true));
        let limits = DeleteLimits::default();
        assert!(
            1 <= limits.allowance(200_000),
            "this must be well within the blast-radius allowance, or the test proves nothing"
        );
        assert!(matches!(
            evaluate(&mut state, &limits, 200_000, 0),
            GuardedDeletes::Blocked
        ));
        let block = state.writeback_blocked.unwrap();
        assert!(block.reason.contains("bulk"), "{}", block.reason);
    }

    /// The soft block is releasable, and the release is scoped to the batch it
    /// was given for.
    #[test]
    fn a_matching_confirm_token_releases_exactly_that_batch() {
        let limits = DeleteLimits::default();
        let mut state = state_with(pending(1, true));
        assert!(matches!(
            evaluate(&mut state, &limits, 100, 0),
            GuardedDeletes::Blocked
        ));
        let token = state.writeback_blocked.clone().unwrap().token;

        // The operator confirms.
        state.writeback_confirm_token = Some(token.clone());
        assert_eq!(allowed(&evaluate(&mut state, &limits, 100, 0)), 1);
        assert!(state.writeback_blocked.is_none());
        assert!(state.writeback_confirm_token.is_none());

        // A confirmation for a batch of one must NOT authorise a batch that has
        // since grown: the token is derived from the batch's content.
        let mut grown = state_with(pending(4, true));
        grown.writeback_blocked = Some(WritebackBlock {
            token: token.clone(),
            reason: "stale".into(),
            deletes: 1,
            at: 0,
        });
        grown.writeback_confirm_token = Some(token);
        assert!(matches!(
            evaluate(&mut grown, &limits, 100, 0),
            GuardedDeletes::Blocked
        ));
    }

    /// Nothing is pushed without provenance, even if a state blob claims it.
    #[test]
    fn a_pending_delete_with_no_external_id_is_dropped_not_pushed() {
        let mut deletes = pending(1, false);
        deletes[0].external_id = String::new();
        let mut state = state_with(deletes);
        assert!(matches!(
            evaluate(&mut state, &DeleteLimits::default(), 100, 0),
            GuardedDeletes::Nothing
        ));
        assert!(state.writeback_pending_deletes.is_empty());
    }

    /// A drain with nothing pending must not clear a block it never evaluated.
    #[test]
    fn an_empty_queue_leaves_an_existing_block_alone() {
        let mut state = state_with(Vec::new());
        state.writeback_blocked = Some(WritebackBlock {
            token: "t".into(),
            reason: "r".into(),
            deletes: 3,
            at: 0,
        });
        assert!(matches!(
            evaluate(&mut state, &DeleteLimits::default(), 10, 0),
            GuardedDeletes::Nothing
        ));
        assert!(state.writeback_blocked.is_some());
    }
}
