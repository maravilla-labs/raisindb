//! The NAMED expected-failure list.
//!
//! Each entry is a template the oracle knows to fail today, why, which phase
//! owns the fix, and a minimal WITNESS history that reproduces it. Random runs
//! tolerate mismatches of a listed template whose detail carries the listed
//! `signature`; `expected_failures_still_fail` replays every witness and
//! asserts that the WRONG result (`witness_result`) is still produced — so a
//! listed template that starts passing FAILS the build, and the list is pruned
//! the day its fixing phase lands. `#[should_panic]` is deliberately not used:
//! it passes on any panic and would mask new regressions in these templates.

use super::checks::{name, Mismatch};
use super::ops::Op;
use super::{replay_witnesses, witnesses};

pub struct ExpectedFailure {
    /// The template name (`checks::name`).
    pub template: &'static str,
    /// A substring every TOLERATED mismatch's detail contains in random runs
    /// (`Some("")` = the whole template, used only where the template itself
    /// is narrow enough to name one defect). `None`: never tolerated — the
    /// generators avoid the shape, and only the witness asserts it.
    pub signature: Option<&'static str>,
    /// The wrong answer the witness must still produce (a detail substring).
    pub witness_result: &'static str,
    /// Why it fails and which phase owns the fix.
    pub reason: &'static str,
    /// A history that reproduces it.
    pub witness: fn() -> Vec<Op>,
    /// Whether the witness runs on stage 3's replica (origin A runs it, the
    /// replica is asked) rather than on a single storage.
    pub replica: bool,
}

pub const EXPECTED_FAILURES: &[ExpectedFailure] = &[
    ExpectedFailure {
        template: name::MERGE_RETRO,
        signature: Some(""),
        witness_result: r#"o001 properties: got {"title": String("beta")}, model {"title": String("alpha")}"#,
        reason: "Merge replays the source branch's entries into the target at their ORIGINAL \
                 revisions (`copy_branch_indexes`), so every target revision taken between the \
                 fork and the merge shows the fork's changes retroactively: time travel on the \
                 target is rewritten by the merge. HEAD and every revision outside that window \
                 are asserted strictly. No phase owns this yet; the fix is merge apply writing \
                 the source's changes at the merge revision (it already does for resolutions).",
        witness: witnesses::merge_rewrites_target_history,
        replica: false,
    },
    ExpectedFailure {
        template: name::NODES_CF,
        signature: None,
        witness_result: r#"a002 properties: got {"title": String("alpha")}, model {"title": String("delta")}"#,
        reason: "replica_in_place_after_ancestor_move: a `versionable=false` node rewritten in \
                 place AFTER an ancestor moved diverges on a replica. A move replicates a \
                 snapshot of every moved node, which on the replica is a NEW node version at \
                 the move's revision (the origin wrote none: descendants' records stay put); the \
                 origin's later in-place write reuses the node's OLD revision, so on the replica \
                 it lands beneath that newer version and is shadowed. Stage 3 generates no \
                 versionable=false nodes until this is fixed (no phase owns it yet; the fix is \
                 the reuse rule taking the node's newest revision across NODES and NODE_PATH, or \
                 the move not minting replica versions for unchanged records).",
        witness: replay_witnesses::volatile_after_ancestor_move,
        replica: true,
    },
];

/// Anomalies (driver-level findings) that are known, by substring. Empty: a
/// write the model accepts and the system refuses is never tolerated.
pub const EXPECTED_ANOMALIES: &[(&str, &str)] = &[];

pub fn is_listed(m: &Mismatch) -> bool {
    EXPECTED_FAILURES
        .iter()
        .any(|e| e.template == m.template && e.signature.is_some_and(|sig| m.detail.contains(sig)))
}

pub fn anomaly_listed(a: &str) -> bool {
    EXPECTED_ANOMALIES.iter().any(|(sig, _)| a.contains(sig))
}
