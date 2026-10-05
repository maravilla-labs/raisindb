//! Debugging and triage aids, all `#[ignore]`d: witness replays, random
//! sweeps that do not stop at the first failure, plan dumps and raw CF dumps.
//! Environment: `ORACLE_WITNESS`, `ORACLE_REPLAY_WITNESS`, `ORACLE_SWEEP`,
//! `ORACLE_DELIVERY`, `ORACLE_VERBOSE`, `ORACLE_DUMP`.

use super::checks::Mismatch;
use super::driver::Run;
use super::*;

/// Debugging aid: `ORACLE_WITNESS=<name> cargo test ... oracle_witness_debug
/// -- --ignored --nocapture` replays one fixed history and prints EVERY
/// mismatch, listed or not.
#[test]
#[ignore]
fn oracle_witness_debug() {
    let wanted = std::env::var("ORACLE_WITNESS").unwrap_or_default();
    let rt = runtime();
    for (name, ops) in witnesses::all() {
        if !wanted.is_empty() && wanted != name {
            continue;
        }
        rt.block_on(async {
            let run = drive(&ops()).await;
            let mismatches = check(&run, |_| true).await;
            let all: Vec<&Mismatch> = mismatches.iter().collect();
            eprintln!("=== {name}\n{}", report(&run, &all));
            if std::env::var("ORACLE_DUMP").is_ok() {
                dump_nodes(&run);
            }
        });
    }
}

/// Triage aid: `ORACLE_SWEEP=<n> cargo test ... oracle_sweep -- --ignored
/// --nocapture` drives `n` random histories WITHOUT stopping at the first
/// failure and prints, per template, how many histories failed it and the
/// shortest failing history.
#[test]
#[ignore]
fn oracle_sweep() {
    use proptest::strategy::{Strategy, ValueTree};
    let n: usize = std::env::var("ORACLE_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let rt = runtime();
    let mut runner = proptest::test_runner::TestRunner::new(config());
    let mut worst: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for _ in 0..n {
        let ops = ops::history()
            .new_tree(&mut runner)
            .expect("tree")
            .current();
        rt.block_on(async {
            let run = drive(&ops).await;
            let mismatches = check(&run, |_| true).await;
            let mut keys: Vec<String> = mismatches
                .iter()
                .filter(|m| !expected::is_listed(m))
                .map(|m| m.template.to_string())
                .collect();
            keys.extend(
                run.anomalies
                    .iter()
                    .filter(|a| !expected::anomaly_listed(a))
                    .map(|a| format!("anomaly:{}", a.split(':').next().unwrap_or(a))),
            );
            keys.sort();
            keys.dedup();
            for k in keys {
                *counts.entry(k.clone()).or_default() += 1;
                let all: Vec<&Mismatch> = mismatches.iter().filter(|m| m.template == k).collect();
                let text = report(&run, &all);
                let len = run.log.len();
                let e = worst.entry(k).or_insert((usize::MAX, String::new()));
                if len < e.0 {
                    *e = (len, text);
                }
            }
        });
    }
    eprintln!("=== sweep of {n}: failing histories per template: {counts:?}");
    for (k, (_, text)) in worst {
        eprintln!("=== shortest for {k}\n{text}");
    }
}

/// Debugging aid: every stored NODES version and every `__updated_at`
/// PROPERTY_INDEX entry of `main`, raw.
fn dump_nodes(run: &Run) {
    let db = run.env.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::NODES).expect("cf");
    let prefix = format!("{}\0{}\0main\0{}\0nodes\0", env::TENANT, env::REPO, env::WS);
    for (k, v) in db
        .prefix_iterator_cf(&cf, prefix.as_bytes())
        .flatten()
        .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
    {
        let rev = raisin_hlc::HLC::decode_descending(&k[k.len() - 16..]).ok();
        let id = String::from_utf8_lossy(&k[prefix.len()..k.len() - 17]).to_string();
        let what = match raisin_rocksdb::decode_node_blob(&v) {
            Ok((n, _)) => format!("updated_at={:?} order_key={}", n.updated_at, n.order_key),
            Err(_) => format!("{:?}", String::from_utf8_lossy(&v)),
        };
        eprintln!("  NODES {id} @{rev:?}: {what}");
    }
    let cf = db
        .cf_handle(raisin_rocksdb::cf::COMPOUND_INDEX)
        .expect("cf");
    let prefix = format!("{}\0{}\0", env::TENANT, env::REPO);
    for (k, v) in db
        .prefix_iterator_cf(&cf, prefix.as_bytes())
        .flatten()
        .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
    {
        eprintln!(
            "  COMPOUND {:?} = {:?}",
            String::from_utf8_lossy(&k[prefix.len()..]),
            String::from_utf8_lossy(&v)
        );
    }
    let cf = db
        .cf_handle(raisin_rocksdb::cf::PROPERTY_INDEX)
        .expect("cf");
    let prefix = format!(
        "{}\0{}\0main\0{}\0prop\0__updated_at\0",
        env::TENANT,
        env::REPO,
        env::WS
    );
    for (k, v) in db
        .prefix_iterator_cf(&cf, prefix.as_bytes())
        .flatten()
        .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
    {
        eprintln!(
            "  PROP {:?} = {:?}",
            String::from_utf8_lossy(&k[prefix.len()..]),
            String::from_utf8_lossy(&v)
        );
    }
}

/// Debugging aid: print the plan of every template's SQL shape.
#[test]
#[ignore]
fn oracle_explain_plans() {
    let rt = runtime();
    rt.block_on(async {
        let run = drive(&witnesses_fixed::compound_after_merge()).await;
        let engine = run.env.engine(env::MAIN);
        let head = run.env.head(env::MAIN).await;
        let rev = format!(" AND __revision = '{head}'");
        for sql in checks::template_shapes(&rev)
            .into_iter()
            .chain(checks::template_shapes(""))
        {
            let plan = env::query(&engine, &format!("EXPLAIN {sql}")).await;
            eprintln!("=== {sql}\n{plan:?}");
        }
    });
}

/// Debugging aid: replay each stage-3 witness (`ORACLE_REPLAY_WITNESS=<name>`
/// for one; `ORACLE_DELIVERY`, `ORACLE_SEED`) and print every replica mismatch.
#[test]
#[ignore]
fn oracle_replay_witness_debug() {
    let wanted = std::env::var("ORACLE_REPLAY_WITNESS").unwrap_or_default();
    let rt = runtime();
    for (name, ops) in replay_witnesses::all() {
        if !wanted.is_empty() && wanted != name {
            continue;
        }
        let seed = std::env::var("ORACLE_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        rt.block_on(async {
            let r = replay::replay(&ops, &[], delivery_from_env(), seed).await;
            let mismatches = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
            let mut per: std::collections::BTreeMap<&str, usize> = Default::default();
            for m in &mismatches {
                *per.entry(m.template).or_default() += 1;
            }
            eprintln!(
                "=== {name}: {} ops, anomalies {:?}, {per:?}",
                r.applied, r.anomalies
            );
            for m in mismatches
                .iter()
                .take(if std::env::var("ORACLE_VERBOSE").is_ok() {
                    8
                } else {
                    2
                })
            {
                eprintln!(
                    "  [{}] {}",
                    m.template,
                    m.detail.chars().take(400).collect::<String>()
                );
            }
        });
    }
}

/// Triage aid for stage 3: `ORACLE_SWEEP=<n>` random replays (causal unless
/// `ORACLE_DELIVERY=permuted|unbuffered`), per-template failure counts and the
/// shortest failing replay for each.
#[test]
#[ignore]
fn oracle_replay_sweep() {
    use proptest::strategy::{Strategy, ValueTree};
    let n: usize = std::env::var("ORACLE_SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let delivery = delivery_from_env();
    let rt = runtime();
    let mut runner = proptest::test_runner::TestRunner::new(config());
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    let mut worst: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for i in 0..n {
        let a = ops::replicable_history()
            .new_tree(&mut runner)
            .expect("tree")
            .current();
        let b = ops::replicable_history()
            .new_tree(&mut runner)
            .expect("tree")
            .current();
        rt.block_on(async {
            let r = replay::replay(&a, &b, delivery, i as u64).await;
            let mismatches = check_snaps(&r.replica, &r.snaps, &r.instants, |_| true).await;
            let mut keys: Vec<String> = mismatches
                .iter()
                .filter(|m| !expected::is_listed(m))
                .map(|m| m.template.to_string())
                .collect();
            keys.extend(r.anomalies.iter().map(|_| "anomaly".to_string()));
            keys.sort();
            keys.dedup();
            for k in keys {
                *counts.entry(k.clone()).or_default() += 1;
                let mine: Vec<&Mismatch> = mismatches.iter().filter(|m| m.template == k).collect();
                let len = r.a.log.len() + r.b.log.len();
                let text = format!(
                    "anomalies {:?}\nA {}\nB {}\nREPLICA {}",
                    r.anomalies,
                    report(&r.a, &[]),
                    report(&r.b, &[]),
                    report(&r.a, &mine).split("unexpected").last().unwrap_or("")
                );
                let e = worst.entry(k).or_insert((usize::MAX, String::new()));
                if len < e.0 {
                    *e = (len, text);
                }
            }
        });
    }
    eprintln!("=== replay sweep of {n} ({delivery:?}): {counts:?}");
    for (k, (_, text)) in worst {
        eprintln!("=== shortest for {k}\n{text}");
    }
}

/// `ORACLE_DELIVERY=causal|permuted|unbuffered` (causal by default).
fn delivery_from_env() -> replay::Delivery {
    match std::env::var("ORACLE_DELIVERY").as_deref() {
        Ok("permuted") => replay::Delivery::Permuted,
        Ok("unbuffered") => replay::Delivery::Unbuffered,
        _ => replay::Delivery::Causal,
    }
}
