//! Phase 0b — the MVCC index oracle.
//!
//! One property test that turns an index or MVCC regression into a shrunk,
//! minimal failing history. It generates histories of 20–60 operations —
//! create, update (string / number / reference / reference nested in an
//! Element or Composite), retype, `versionable=false` updates, delete (with
//! and without cascade), move subtree, rename, reorder, copy tree, set
//! translation, RESTORE, several writes in ONE transaction, and fork → edits on
//! both sides → merge with KeepOurs / KeepTheirs resolutions — and drives them
//! through the real write funnels.
//!
//! The truth it compares against is an INDEPENDENT reference model
//! (`model.rs`) advanced by the same op log: parent ids and explicit sibling
//! order, never `cf::NODES`, NODE_PATH or ORDERED_CHILDREN. At every revision
//! a branch HEAD reached, every template (`checks/`) is asked its question at
//! that revision and its answer compared with the model's.
//!
//! - **Stage 1** (`oracle_histories`): the histories above.
//! - **Stage 2** (`oracle_history_gc`): history GC with a random cutoff plus a
//!   tag pin; every retained revision must answer exactly as before.
//! - **Stage 2b** (`oracle_collapse_runs`, plan Phase 9): run-collapse GC,
//!   alone or after retention GC; every retained revision must answer exactly
//!   as before.
//! - **Stage 3** (`oracle_replication_replay`): two origins' oplogs,
//!   interleaved and permuted, replayed into a third storage.
//!
//! Known failures are on a NAMED list (`expected.rs`) whose wrong results are
//! asserted by `expected_failures_still_fail`; a listed template that starts
//! passing fails that test, so the list is pruned the day the fix lands.
//!
//! Cases: `PROPTEST_CASES` (default 16). History length: `ORACLE_OPS`
//! (`min..max`, default `20..40`; the plan's full range is `20..60`).
//!
//! ```bash
//! cargo test -p raisin-sql-execution --test all mvcc_index_oracle
//! PROPTEST_CASES=200 ORACLE_OPS=20..60 cargo test -p raisin-sql-execution --test all mvcc_index_oracle
//! ```

mod checks;
mod collapse;
mod content_ops;
mod copy_restore;
mod debug;
mod driver;
pub(crate) mod env;
mod expected;
mod fork_edit;
mod gc;
mod merge;
mod merge_model;
mod model;
mod ops;
mod replay;
mod replay_legacy;
mod replay_witnesses;
mod stages;
mod tx_ops;
mod witnesses;
mod witnesses_fixed;
mod witnesses_review;
mod write_ops;

use checks::{Checker, Mismatch};
use driver::Run;
use env::Env;
use model::{apply_taint, Snapshot};
use ops::Op;
use proptest::prelude::*;
use std::collections::{BTreeSet, HashMap};

pub fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    ProptestConfig {
        cases,
        max_shrink_iters: if std::env::var("ORACLE_NO_SHRINK").is_ok() {
            0
        } else {
            std::env::var("ORACLE_SHRINK")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(48)
        },
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

/// Drive `ops` on a fresh storage.
pub async fn drive(ops: &[Op]) -> Run {
    let mut run = Run::new(Env::new(None).await).await;
    for op in ops {
        run.apply(op).await;
    }
    apply_taint(&mut run.snaps, &run.writes);
    model::apply_overlay_gaps(&mut run.snaps, &run.overlay_events);
    run
}

/// Check every snapshot `keep` admits against the system.
pub async fn check(run: &Run, keep: impl Fn(&Snapshot) -> bool) -> Vec<Mismatch> {
    let mut out = check_snaps(&run.env, &run.snaps, &run.instants, keep).await;
    // Fork windows, asserted just before their merges (`merge.rs`).
    out.extend(run.pre_merge.iter().cloned());
    out
}

/// Check `snaps` against `env` (stage 3 checks a replica against snapshots
/// the origins recorded).
pub async fn check_snaps(
    env: &Env,
    snaps: &[Snapshot],
    instants: &[(u64, String)],
    keep: impl Fn(&Snapshot) -> bool,
) -> Vec<Mismatch> {
    let mut checker = Checker::new(env, instants);
    let mut seen: HashMap<String, BTreeSet<String>> = HashMap::new();
    for s in snaps {
        let paths = seen.entry(s.branch.clone()).or_default();
        paths.extend(s.tree.nodes.keys().map(|i| s.tree.path(i)));
    }
    checker.seen_paths = seen;
    for (i, s) in snaps.iter().enumerate() {
        if !keep(s) {
            continue;
        }
        let newest = !snaps[i + 1..].iter().any(|t| t.branch == s.branch);
        checker.check(s, newest).await;
    }
    checker.out
}

/// The failure report: the op log, anomalies, and the first mismatches.
pub fn report(run: &Run, mismatches: &[&Mismatch]) -> String {
    let mut out = String::from("history:\n");
    for l in &run.log {
        out.push_str(&format!("  {l}\n"));
    }
    if !run.anomalies.is_empty() {
        out.push_str("anomalies:\n");
        for a in &run.anomalies {
            out.push_str(&format!("  {a}\n"));
        }
    }
    out.push_str(&format!("{} unexpected mismatch(es):\n", mismatches.len()));
    let mut per: std::collections::BTreeMap<&str, usize> = Default::default();
    for m in mismatches {
        *per.entry(m.template).or_default() += 1;
    }
    out.push_str(&format!("  by template: {per:?}\n"));
    let verbose = std::env::var("ORACLE_VERBOSE").is_ok();
    let mut shown: std::collections::BTreeMap<&str, usize> = Default::default();
    for m in mismatches.iter() {
        let n = shown.entry(m.template).or_default();
        *n += 1;
        if *n > if verbose { 6 } else { 2 } {
            continue;
        }
        out.push_str(&format!(
            "  [{}] {}@{} (after op #{}): {}\n",
            m.template, m.branch, m.head, m.op, m.detail
        ));
    }
    out
}

/// Triage: `ORACLE_ONLY=<template>[,<template>]` fails on those templates
/// alone (`anomaly` selects driver anomalies), so shrinking minimizes ONE
/// finding instead of whichever the history happens to hit first.
pub fn selected(template: &str) -> bool {
    match std::env::var("ORACLE_ONLY") {
        Ok(only) => only.split(',').any(|x| x == template),
        Err(_) => true,
    }
}

/// Fail on anything not on the expected-failure list.
pub fn verdict(run: &Run, mismatches: &[Mismatch]) -> Result<(), String> {
    // Triage: `ORACLE_ONLY=<template>[,<template>]` fails on those templates
    // alone (`anomaly` selects driver anomalies), so shrinking minimizes ONE
    // finding instead of whichever the history happens to hit first.
    let unexpected: Vec<&Mismatch> = mismatches
        .iter()
        .filter(|m| !expected::is_listed(m) && selected(m.template))
        .collect();
    let anomalies: Vec<&String> = run
        .anomalies
        .iter()
        .filter(|a| !expected::anomaly_listed(a) && selected("anomaly"))
        .collect();
    if unexpected.is_empty() && anomalies.is_empty() {
        Ok(())
    } else {
        Err(report(run, &unexpected))
    }
}
