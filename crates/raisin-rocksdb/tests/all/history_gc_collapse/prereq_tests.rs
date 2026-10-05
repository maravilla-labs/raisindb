//! Collapse touches a CF on a branch only after the repairs that correct it
//! have completed there, so a repair's historical tombstone can never land
//! inside a run collapse already shortened.

use super::env::{node, options, Env, REPO, TENANT, WS};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_rocksdb::management::async_indexing::repair::{mark_repairs_pending, RepairKind};
use raisin_rocksdb::{cf, keys};
use std::collections::BTreeMap;

/// Delete every ORDERED_CHILDREN tombstone of `child` under `parent` — the
/// state the old name-keyed delete tombstoner left (see `order_repair_test`).
fn forget_order_tombstones(env: &Env, parent: &str, child: &str) {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, "main", WS, parent);
    let db = env.storage.db();
    let handle = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    let doomed: Vec<Vec<u8>> = db
        .prefix_iterator_cf(handle, &prefix)
        .map(|item| item.unwrap())
        .take_while(|(key, _)| key.starts_with(&prefix))
        .filter(|(key, value)| {
            keys::is_tombstone_value(value) && key.ends_with(format!("\0{child}").as_bytes())
        })
        .map(|(key, _)| key.to_vec())
        .collect();
    for key in doomed {
        db.delete_cf(handle, key).unwrap();
    }
}

/// `child`'s live ORDERED_CHILDREN entry under `parent`: `(label, value)`.
fn live_entry(env: &Env, parent: &str, child: &str) -> (String, Vec<u8>) {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, "main", WS, parent);
    for (key, value) in env.raw(cf::ORDERED_CHILDREN, "main") {
        if key.starts_with(&prefix)
            && key.ends_with(format!("\0{child}").as_bytes())
            && !keys::is_tombstone_value(&value)
        {
            let rest = &key[prefix.len()..];
            let end = rest.iter().position(|b| *b == 0).unwrap();
            return (String::from_utf8_lossy(&rest[..end]).into_owned(), value);
        }
    }
    panic!("{child} has no live entry under {parent}");
}

/// The plan's hazard: `c` is re-written under one label, deleted with its
/// tombstone lost, and a stale live entry for it lands later at the same
/// label (a verbatim merge copy). Collapsing first would fold that stale
/// entry into the run and the repair's tombstone would then reshape history.
/// Returns the HEAD after every step.
async fn script(env: &Env) -> Result<Vec<HLC>> {
    let mut revs = vec![env.add("main", node("p", "/p", "parent")).await?];
    revs.push(env.add("main", node("c", "/p/c", "c0")).await?);
    for v in 1..=3 {
        revs.push(env.put("main", node("c", "/p/c", &format!("c{v}"))).await?);
    }
    let (label, value) = live_entry(env, "p", "c");
    revs.push(env.delete("main", "c").await?);
    forget_order_tombstones(env, "p", "c");
    revs.push(env.add("main", node("d", "/p/d", "d0")).await?);
    let stale_at = revs[revs.len() - 1];
    let db = env.storage.db();
    db.put_cf(
        db.cf_handle(cf::ORDERED_CHILDREN).unwrap(),
        keys::ordered_child_key_versioned(TENANT, REPO, "main", WS, "p", &label, &stale_at, "c"),
        value,
    )
    .unwrap();
    for v in 1..=2 {
        revs.push(env.put("main", node("d", "/p/d", &format!("d{v}"))).await?);
    }
    Ok(revs)
}

fn ordered(env: &Env, revs: &[HLC]) -> Vec<BTreeMap<Vec<u8>, Vec<u8>>> {
    revs.iter()
        .map(|r| env.decided(cf::ORDERED_CHILDREN, "main", r))
        .collect()
}

#[tokio::test]
async fn collapse_refuses_while_repair_pending() -> Result<()> {
    let env = Env::new().await?;
    script(&env).await?;
    let before = env.raw(cf::ORDERED_CHILDREN, "main");

    // No repair has run on this node: ORDERED_CHILDREN is refused, the CFs
    // without a prerequisite are collapsed.
    let first = env.collapse(Some("main"), options()).await?;
    assert!(!first[0].completed);
    assert!(
        first[0]
            .collapse
            .refused
            .iter()
            .any(|r| r.starts_with(cf::ORDERED_CHILDREN)
                && r.contains("ordered_children")
                && r.contains("node_path")),
        "{:?}",
        first[0].collapse
    );
    assert!(first[0].collapse.column_families[cf::PROPERTY_INDEX].deleted > 0);
    assert_eq!(env.raw(cf::ORDERED_CHILDREN, "main"), before);

    // One repair done is not enough.
    env.repair(Some("main"), RepairKind::OrderedChildren, options())
        .await?;
    let second = env.collapse(Some("main"), options()).await?;
    assert!(second[0].collapse.refused[0].contains("node_path"));
    assert!(!second[0].collapse.refused[0].contains("ordered_children,"));

    env.prerequisites(Some("main")).await?;
    let third = env.collapse(Some("main"), options()).await?;
    assert!(third[0].completed, "{:?}", third[0].collapse);

    // A checkpoint ingest owes the repairs again: refused until they re-ran.
    mark_repairs_pending(
        env.storage.db(),
        TENANT,
        REPO,
        "local",
        &[RepairKind::OrderedChildren, RepairKind::NodePath],
    )?;
    let after_ingest = env.collapse(Some("main"), options()).await?;
    assert!(!after_ingest[0].completed);
    assert!(after_ingest[0].collapse.refused[0].starts_with(cf::ORDERED_CHILDREN));
    Ok(())
}

#[tokio::test]
async fn repair_then_collapse_equals_collapse_then_repair() -> Result<()> {
    let (a, b) = (Env::new().await?, Env::new().await?);
    let revs_a = script(&a).await?;
    let revs_b = script(&b).await?;

    // A: repair, then collapse.
    a.prerequisites(Some("main")).await?;
    let repaired = ordered(&a, &revs_a);
    let ra = a.collapse(Some("main"), options()).await?;
    assert!(ra[0].completed);

    // B: collapse first (ORDERED_CHILDREN refused), repair, collapse.
    let rb = b.collapse(Some("main"), options()).await?;
    assert!(rb[0].collapse.refused[0].starts_with(cf::ORDERED_CHILDREN));
    b.prerequisites(Some("main")).await?;
    let rb = b.collapse(Some("main"), options()).await?;
    assert!(rb[0].completed);

    // Collapse changed nothing the repaired data answers, at any revision.
    assert_eq!(ordered(&a, &revs_a), repaired);
    // Both orders end in the same answers and the same entries (labels may
    // carry a per-database suffix, so compare the children listed).
    let listed = |m: &BTreeMap<Vec<u8>, Vec<u8>>| -> Vec<String> {
        let mut out: Vec<String> = m
            .keys()
            .map(|g| {
                let at = g.iter().rposition(|b| *b == 0).unwrap();
                String::from_utf8_lossy(&g[at + 1..]).into_owned()
            })
            .collect();
        out.sort();
        out
    };
    let (la, lb): (Vec<_>, Vec<_>) = (
        ordered(&a, &revs_a).iter().map(listed).collect(),
        ordered(&b, &revs_b).iter().map(listed).collect(),
    );
    assert_eq!(la, lb);
    assert_eq!(
        a.raw(cf::ORDERED_CHILDREN, "main").len(),
        b.raw(cf::ORDERED_CHILDREN, "main").len()
    );
    // And the repair's answers are the right ones: c is listed while it
    // existed, not after its delete, and d stays listed.
    let live = |m: &BTreeMap<Vec<u8>, Vec<u8>>, child: &str| {
        m.keys()
            .any(|g| g.ends_with(format!("\0{child}").as_bytes()))
    };
    let at = ordered(&a, &revs_a);
    assert!(live(&at[2], "c") && live(&at[4], "c"));
    assert!(!live(&at[5], "c"), "deleted c is listed again");
    assert!(!live(at.last().unwrap(), "c"));
    assert!(live(at.last().unwrap(), "d"));
    Ok(())
}

#[tokio::test]
async fn merge_from_unrepaired_branch_rearms_the_ordered_children_prerequisite() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("p", "/p", "parent")).await?;
    env.add("main", node("c", "/p/c", "c0")).await?;
    // feature forks before any repair: it has no repair records of its own.
    env.fork("feature", "main").await?;
    env.prerequisites(Some("main")).await?;
    let ready = env.collapse(Some("main"), options()).await?;
    assert!(ready[0].completed, "{:?}", ready[0].collapse);

    env.add("feature", node("d", "/p/d", "d0")).await?;
    env.put("main", node("c", "/p/c", "c1")).await?;
    let merge = || async {
        env.storage
            .branches_impl()
            .merge_branches(
                TENANT,
                REPO,
                "main",
                "feature",
                raisin_context::MergeStrategy::ThreeWay,
                "merge",
                "test-user",
            )
            .await
    };
    let merged = merge().await?;
    assert!(merged.success && merged.conflicts.is_empty(), "{merged:?}");

    // main now holds feature's unrepaired ORDERED_CHILDREN history.
    let after = env.collapse(Some("main"), options()).await?;
    assert!(!after[0].completed);
    assert!(
        after[0]
            .collapse
            .refused
            .iter()
            .any(|r| { r.starts_with(cf::ORDERED_CHILDREN) && r.contains("ordered_children") }),
        "{:?}",
        after[0].collapse
    );

    // Repaired everywhere, a merge from a repaired source keeps `done`.
    env.prerequisites(None).await?;
    env.add("feature", node("e", "/p/e", "e0")).await?;
    env.put("main", node("c", "/p/c", "c2")).await?;
    let merged = merge().await?;
    assert!(merged.success && merged.conflicts.is_empty(), "{merged:?}");
    let clean = env.collapse(Some("main"), options()).await?;
    assert!(clean[0].completed, "{:?}", clean[0].collapse);
    Ok(())
}
