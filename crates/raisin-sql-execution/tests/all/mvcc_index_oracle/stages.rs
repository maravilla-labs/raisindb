//! The oracle's tests: stage 1 (random histories), stage 2 (history GC),
//! stage 3 (replication replay), the expected-failure witnesses, and every
//! fixed regression witness.

use super::checks::Mismatch;
use super::*;
use proptest::prelude::*;

/// Stage 3's verdict: origin anomalies, replay failures and replica
/// mismatches, with both origins' histories.
pub fn replay_verdict(
    r: &replay::Replay,
    delivery: replay::Delivery,
    mismatches: &[Mismatch],
) -> Result<(), String> {
    let a = verdict(&r.a, &[]);
    let b = verdict(&r.b, &[]);
    let replica: Vec<&Mismatch> = mismatches
        .iter()
        .filter(|m| !expected::is_listed(m) && selected(m.template))
        .collect();
    let anomalies = r.anomalies.iter().any(|_| selected("anomaly"));
    if a.is_ok() && b.is_ok() && replica.is_empty() && !anomalies {
        return Ok(());
    }
    let mut out = format!("{delivery:?} replay of {} operations\n", r.applied);
    for a in &r.anomalies {
        out.push_str(&format!("  {a}\n"));
    }
    out.push_str(&format!("ORIGIN A {}\n", report(&r.a, &[])));
    out.push_str(&format!("ORIGIN B {}\n", report(&r.b, &[])));
    out.push_str(&format!(
        "REPLICA {}",
        report(&r.a, &replica)
            .split("unexpected")
            .last()
            .unwrap_or("")
    ));
    Err(out)
}

proptest! {
    #![proptest_config(config())]

    /// Stage 1: every template at every recorded revision equals the model.
    #[test]
    fn oracle_histories(ops in ops::history()) {
        let rt = runtime();
        let outcome = rt.block_on(async {
            let run = drive(&ops).await;
            let mismatches = check(&run, |_| true).await;
            if std::env::var("ORACLE_VERBOSE").is_ok() {
                eprintln!(
                    "skip-unchanged: {} entries skipped so far",
                    raisin_rocksdb::indexing::skipped_unchanged_entries()
                );
            }
            verdict(&run, &mismatches)
        });
        prop_assert!(outcome.is_ok(), "{}", outcome.unwrap_err());
    }

    /// Stage 3: two origins' oplogs, interleaved (causally, or permuted
    /// through the causal buffer; permuted and applied unbuffered on request),
    /// replayed into a replica; every template at every recorded revision
    /// equals the union of the origins' models.
    #[test]
    fn oracle_replication_replay(
        ops_a in ops::replicable_history(),
        ops_b in ops::replicable_history(),
        // Unbuffered is opt-in (`ORACLE_DELIVERY=unbuffered`) until the
        // apply path can place an op whose old parent has not arrived yet —
        // see `replay.rs`; every op kind is replayed unbuffered by
        // `replay_witnesses_pass_out_of_order`.
        mode in 0u8..2,
        seed in any::<u64>(),
    ) {
        let rt = runtime();
        let outcome = rt.block_on(async {
            let mode = match std::env::var("ORACLE_DELIVERY").as_deref() {
                Ok("causal") => 0,
                Ok("permuted") => 1,
                Ok("unbuffered") => 2,
                _ => mode,
            };
            let delivery = match mode {
                0 => replay::Delivery::Causal,
                1 => replay::Delivery::Permuted,
                _ => replay::Delivery::Unbuffered,
            };
            let r = replay::replay(&ops_a, &ops_b, delivery, seed).await;
            let mismatches = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
            replay_verdict(&r, delivery, &mismatches)
        });
        prop_assert!(outcome.is_ok(), "{}", outcome.unwrap_err());
    }

    /// Stage 2: history GC with a random cutoff plus a tag pin; every
    /// retained revision answers exactly as before.
    #[test]
    fn oracle_history_gc(ops in ops::history(), pin in any::<usize>(), keep in 1u64..12) {
        let rt = runtime();
        let outcome = rt.block_on(async {
            let run = drive(&ops).await;
            let gc = gc::collect(&run, pin, keep).await;
            if std::env::var("ORACLE_VERBOSE").is_ok() {
                let kept = run.snaps.iter().filter(|s| gc.retains(s)).count();
                eprintln!(
                    "gc: keep {keep}: {} versions deleted, {kept}/{} snapshots retained",
                    gc.versions_deleted,
                    run.snaps.len()
                );
            }
            let mismatches = check(&run, |s| gc.retains(s)).await;
            verdict(&run, &mismatches).map_err(|e| {
                format!(
                    "after GC (keep {keep}, pinned {}, cutoffs {:?}, {} versions deleted):\n{e}",
                    gc.pinned, gc.cutoffs, gc.versions_deleted
                )
            })
        });
        prop_assert!(outcome.is_ok(), "{}", outcome.unwrap_err());
    }

    /// Stage 2b (plan Phase 9): run-collapse GC, alone or after retention GC
    /// with a tag pin; every revision retention kept answers exactly as
    /// before — collapse itself forgets nothing.
    #[test]
    fn oracle_collapse_runs(
        ops in ops::history(),
        pin in any::<usize>(),
        keep in proptest::option::of(1u64..12),
    ) {
        let rt = runtime();
        let outcome = rt.block_on(async {
            let run = drive(&ops).await;
            let gc = match keep {
                Some(keep) => Some(gc::collect(&run, pin, keep).await),
                None => None,
            };
            let deleted = collapse::collapse(&run).await;
            if std::env::var("ORACLE_VERBOSE").is_ok() {
                eprintln!("collapse: {deleted} versions deleted (retention {keep:?})");
            }
            let mismatches = check(&run, |s| gc.as_ref().is_none_or(|g| g.retains(s))).await;
            verdict(&run, &mismatches).map_err(|e| {
                format!("after collapse ({deleted} versions deleted, retention {keep:?}):\n{e}")
            })
        });
        prop_assert!(outcome.is_ok(), "{}", outcome.unwrap_err());
    }
}

/// Every listed expected failure still fails, with the listed WRONG result.
/// A listed template whose witness now passes fails this test: prune it.
#[test]
fn expected_failures_still_fail() {
    let rt = runtime();
    for e in expected::EXPECTED_FAILURES {
        let outcome = rt.block_on(async {
            let (run, mismatches) = if e.replica {
                let r = replay::replay(&(e.witness)(), &[], replay::Delivery::Causal, 7).await;
                let m = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
                (r.a, m)
            } else {
                let run = drive(&(e.witness)()).await;
                let m = check(&run, |_| true).await;
                (run, m)
            };
            let mine: Vec<&Mismatch> = mismatches
                .iter()
                .filter(|m| m.template == e.template)
                .collect();
            if mine.is_empty() {
                return Err(format!(
                    "expected failure `{}` unexpectedly PASSES on its witness — remove it from \
                     expected::EXPECTED_FAILURES (and update the plan).\n{}",
                    e.template,
                    report(&run, &mine)
                ));
            }
            if !mine.iter().any(|m| m.detail.contains(e.witness_result)) {
                return Err(format!(
                    "expected failure `{}` fails DIFFERENTLY than listed (wanted `{}`):\n{}",
                    e.template,
                    e.witness_result,
                    report(&run, &mine)
                ));
            }
            Ok(())
        });
        if let Err(msg) = outcome {
            panic!("{msg}");
        }
    }
}

/// Every fixed witness answers cleanly (the regression tests for the bugs
/// the oracle found), and every expected failure's witness fails ONLY in its
/// listed way.
#[test]
fn regression_witnesses_pass() {
    let rt = runtime();
    for (name, ops) in witnesses::all() {
        let outcome = rt.block_on(async {
            let run = drive(&ops()).await;
            let mismatches = check(&run, |_| true).await;
            verdict(&run, &mismatches)
        });
        if let Err(report) = outcome {
            panic!("witness `{name}`:\n{report}");
        }
    }
}

/// Every stage-3 witness (one operation kind each) replicates cleanly, apart
/// from listed expected failures; a witness listed as never tolerated
/// (`signature: None`) is asserted by `expected_failures_still_fail` instead.
#[test]
fn replay_witnesses_pass() {
    let rt = runtime();
    for (name, ops) in replay_witnesses::all() {
        if replay_witnesses::NEVER_TOLERATED.contains(&name) {
            continue;
        }
        let outcome = rt.block_on(async {
            let r = replay::replay(&ops, &[], replay::Delivery::Causal, 7).await;
            let mismatches = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
            replay_verdict(&r, replay::Delivery::Causal, &mismatches)
        });
        if let Err(report) = outcome {
            panic!("replay witness `{name}`:\n{report}");
        }
    }
}

/// Every stage-3 witness again, its oplog shuffled and applied WITHOUT the
/// causal buffer (several seeds): the replica must converge on the same
/// answers whatever order the ops land in. Covers the out-of-order branch of
/// the apply path — the at-or-before baseline, the relabel tombstone,
/// `tombstone_superseded_by_newer`, and two in-place writes at one revision
/// arriving newest first.
#[test]
fn replay_witnesses_pass_out_of_order() {
    let rt = runtime();
    for (name, ops) in replay_witnesses::all() {
        if replay_witnesses::NEVER_TOLERATED.contains(&name) {
            continue;
        }
        for seed in [1, 2, 3, 11] {
            let outcome = rt.block_on(async {
                let delivery = replay::Delivery::Unbuffered;
                let r = replay::replay(&ops, &[], delivery, seed).await;
                let mismatches = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
                replay_verdict(&r, delivery, &mismatches)
            });
            if let Err(report) = outcome {
                panic!("replay witness `{name}` (unbuffered, seed {seed}):\n{report}");
            }
        }
    }
}
