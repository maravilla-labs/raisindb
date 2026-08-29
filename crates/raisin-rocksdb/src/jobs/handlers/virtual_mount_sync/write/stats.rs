//! The drain's receipt: every outcome one writeback pass can have, and the
//! operator-facing summary it leaves on the mount.
//!
//! Its own module because it is DATA the whole `write` module shares — the
//! update loop in [`super`], `deletes`, `create` and `submit` all count into the
//! same struct, and `write_*_tests` assert on it. Nothing here decides anything,
//! so no phase owns it and none of them are change-coupled to it.
//!
//! The counters are deliberately fine-grained and deliberately not folded
//! together; each field's doc says which silence it exists to break.

use super::super::config::DrainSummary;

/// Outcome of one drain, for logging and for the run record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DrainStats {
    pub pushed: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Candidates whose remote object the adapter reported as no longer there.
    ///
    /// Counted apart from both `pushed` and `skipped` on purpose. It is not a
    /// push (nothing reached the provider and nothing was baselined) and it is
    /// not a converged no-op (the edit is still pending, and always will be for
    /// this external id). Folding it into either is what let a `null` result be
    /// reported as a completed push.
    pub gone: usize,
    pub first_error: Option<String>,
    /// The wall-clock budget ran out with candidates still pending.
    ///
    /// Recorded rather than inferred: a drain that stops halfway leaves the
    /// remaining edits pending and looks, from the outside, exactly like a drain
    /// that had nothing to do. That silence is the same class of invisibility as
    /// a sync that never renews its lease.
    pub truncated: bool,
    /// An operator's Stop landed mid-drain and the rest was abandoned.
    pub stopped: bool,
    /// A push failed with a CONFIG error — a missing OAuth scope, a mount
    /// pointed at something that cannot accept writes.
    ///
    /// Terminal for the whole drain, not for the one candidate. The condition is
    /// a property of the MOUNT, so every remaining push would fail identically,
    /// and the edits are not bad data: they stay pending and land untouched once
    /// the configuration is fixed.
    pub misconfigured: bool,
    /// Candidates never attempted because the drain ended early.
    ///
    /// Only ever non-zero alongside `truncated`, `stopped` or `blocked`. Carried
    /// out of the drain on [`MountState::last_drain`] because it is the one
    /// number that separates a mount that is caught up from one that is falling
    /// behind, and a clean `outcome: ok` says nothing about it.
    pub pending: usize,
    /// Deletes actually pushed to the provider.
    pub deleted: usize,
    /// Deletes deliberately NOT pushed, under `delete_policy: detach`. Counted
    /// apart from `deleted` because the remote object is still there — see
    /// [`deletes`] for why that has to be visible rather than implied.
    pub detached: usize,
    /// A blast-radius rail refused this run's deletes and parked every pending
    /// intent. Nothing was sent and nothing was lost; reads were unaffected.
    pub blocked: bool,
    /// Updates withheld whole because `move_policy: reject` and the node's
    /// location field had changed locally.
    ///
    /// Counted apart from `skipped` because a skip is a converged no-op and this
    /// is an edit the mount is refusing to make. Folding them together would let
    /// a mount report "nothing to do" while every write it owes is being
    /// refused — the same silence a `gone` used to have.
    pub rejected: usize,
    /// Pushes the provider refused as conflicts and the policy ABANDONED.
    ///
    /// Under the default `remote_wins` this is a count of local edits thrown
    /// away, so it has to be visible: a mount whose users keep losing edits
    /// otherwise reports the same `outcome: ok` as one that is converged.
    pub conflicted: usize,
    /// Conflicts left for a human — the `error` policy, or a resolver that
    /// parked, threw or answered something unrecognized.
    pub parked: usize,
    /// The first park reason, verbatim, for `writeback_last_error`.
    pub first_park: Option<String>,

    // ---- `submit` only (§5). Deliberately NOT folded into the fields above ----
    //
    // A command is not an edit and the two must not share a counter. "Pushed"
    // means a local value now matches the provider and can be re-derived if it
    // does not; "submitted" means an email left the building. An operator
    // reading one number for both has no way to tell a converging mount from one
    // that has sent thirty messages it was not supposed to.
    /// Commands the provider accepted. Terminal.
    pub submitted: usize,
    /// Commands explicitly refused BEFORE the provider acted (`rate_limited`)
    /// and returned to `queued`. The only outcome here that is tried again.
    pub requeued: usize,
    /// Commands terminally `failed` — definitively not sent.
    pub abandoned: usize,
    /// Commands parked at `unknown`: they may or may not have been issued, and
    /// nothing but a person will ever retry them.
    ///
    /// The number that matters most on this path, which is why it is its own.
    pub unresolved: usize,
}

impl DrainStats {
    /// The operator-facing receipt this drain leaves on the mount.
    pub(super) fn summary(&self) -> DrainSummary {
        DrainSummary {
            pushed: self.pushed as u64,
            pending: self.pending as u64,
            gone: self.gone as u64,
            deleted: self.deleted as u64,
            detached: self.detached as u64,
            blocked: self.blocked,
            failed: self.failed as u64,
            truncated: self.truncated,
            stopped: self.stopped,
            rejected: self.rejected as u64,
            conflicts: self.conflicted as u64,
            parked: self.parked as u64,
            submitted: self.submitted as u64,
            requeued: self.requeued as u64,
            abandoned: self.abandoned as u64,
            unresolved: self.unresolved as u64,
        }
    }
}
