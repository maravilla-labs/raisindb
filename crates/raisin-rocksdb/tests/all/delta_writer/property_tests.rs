//! The PROPERTY_INDEX / UNIQUE / REFERENCE deltas on the origin write paths.

use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type;
use raisin_error::Result;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_storage::{NodeRepository, ReferenceIndexRepository, Storage};

fn many_props(changed: &str) -> Vec<(String, String)> {
    let mut props: Vec<(String, String)> = (0..29)
        .map(|i| (format!("p{i:02}"), format!("v{i}")))
        .collect();
    props.push(("title".to_string(), changed.to_string()));
    props
}

fn page(id: &str, path: &str, props: &[(String, String)]) -> raisin_models::nodes::Node {
    let borrowed: Vec<(&str, &str)> = props
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    node(id, path, &borrowed)
}

/// Harness H's shape: one property changed on a node with P = 30 properties.
/// With the delta writer the update stages only what changed — the title
/// (tombstone + put) and `__updated_at` (tombstone + put) — instead of every
/// entry the node indexes; without skip it re-puts all of them.
#[tokio::test]
async fn unchanged_update_writes_no_entries_for_unchanged_props() -> Result<()> {
    let mut counts = Vec::new();
    for skip in [false, true] {
        let env = Env::new(skip).await?;
        env.add("main", page("n", "/n", &many_props("before")))
            .await?;
        let created = env.newest_revision("main", "n");
        let at_create = env.property_keys_at("main", "n", &created);
        env.put("main", page("n", "/n", &many_props("after")))
            .await?;
        let updated = env.newest_revision("main", "n");
        assert!(updated > created);
        let at_update = env.property_keys_at("main", "n", &updated);
        counts.push((skip, at_create, at_update));

        // Reads are identical either way.
        assert_eq!(env.indexed("main", "title", "after", None).await?, ["n"]);
        assert!(env
            .indexed("main", "title", "before", None)
            .await?
            .is_empty());
        assert_eq!(env.indexed("main", "p07", "v7", None).await?, ["n"]);
        assert_eq!(
            env.indexed("main", "title", "before", Some(&created))
                .await?,
            ["n"]
        );
        assert_eq!(
            env.indexed("main", "p07", "v7", Some(&created)).await?,
            ["n"]
        );
    }
    let (_, full_create, full_update) = counts[0];
    let (_, _, delta_update) = counts[1];
    assert!(full_update >= full_create, "{counts:?}");
    assert_eq!(
        delta_update, 4,
        "title and __updated_at, each a tombstone and a put: {counts:?}"
    );
    Ok(())
}

/// A value written once and skipped by two later writes answers at every
/// revision: before the first write nothing, between the writes from the
/// entry the create left.
#[tokio::test]
async fn time_travel_between_two_writes_with_skip() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("n", "/n", &[("a", "1"), ("b", "1")]))
        .await?;
    let r1 = env.newest_revision("main", "n");
    env.put("main", node("n", "/n", &[("a", "1"), ("b", "2")]))
        .await?;
    let r2 = env.newest_revision("main", "n");
    env.put("main", node("n", "/n", &[("a", "1"), ("b", "3")]))
        .await?;
    let r3 = env.newest_revision("main", "n");
    assert!(r1 < r2 && r2 < r3);
    // `a` was written once, at r1.
    assert_eq!(env.property_keys_at("main", "n", &r2) as i64 - 4, 0);

    for (at, b_live) in [(r1, "1"), (r2, "2"), (r3, "3")] {
        assert_eq!(
            env.indexed("main", "a", "1", Some(&at)).await?,
            ["n"],
            "{at}"
        );
        for b in ["1", "2", "3"] {
            let found = env.indexed("main", "b", b, Some(&at)).await?;
            assert_eq!(found.is_empty(), b != b_live, "b={b} at {at}");
        }
        let n = env
            .storage
            .nodes()
            .get(env.scope("main"), "n", Some(&at))
            .await?
            .expect("node at revision");
        assert_eq!(
            n.properties.get("b"),
            Some(&PropertyValue::String(b_live.into()))
        );
    }
    assert_eq!(env.indexed("main", "a", "1", None).await?, ["n"]);
    assert_eq!(env.indexed("main", "__name", "n", None).await?, ["n"]);
    Ok(())
}

#[tokio::test]
async fn unique_violation_still_detected_after_unchanged_update() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Unique", Some("sku"), None).await?;
    let mut a = node("a", "/a", &[("sku", "S-1"), ("title", "one")]);
    a.node_type = "test:Unique".to_string();
    env.add("main", a.clone()).await?;
    // An update that leaves `sku` alone (its claim is re-put anyway).
    a.properties
        .insert("title".into(), PropertyValue::String("two".into()));
    env.put("main", a.clone()).await?;

    // A second node claiming the same value is still refused...
    let mut b = node("b", "/b", &[("sku", "S-1")]);
    b.node_type = "test:Unique".to_string();
    let ctx = env.tx("main").await?;
    let refused = ctx.add_node(WS, &b).await;
    assert!(refused.is_err(), "duplicate unique value accepted");
    drop(ctx);
    // ...and `a` itself may keep and re-save it.
    env.put("main", a.clone()).await?;
    // Releasing the value frees it.
    a.properties
        .insert("sku".into(), PropertyValue::String("S-2".into()));
    env.put("main", a).await?;
    env.add("main", b).await?;
    Ok(())
}

fn reference_to(target: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: target.to_string(),
        workspace: WS.to_string(),
        path: format!("/{target}"),
    })
}

#[tokio::test]
async fn reference_removed_is_tombstoned_unchanged_is_reput() -> Result<()> {
    let env = Env::new(true).await?;
    for target in ["t1", "t2"] {
        env.add("main", node(target, &format!("/{target}"), &[]))
            .await?;
    }
    let mut src = node("src", "/src", &[("title", "x")]);
    src.properties.insert("keep".into(), reference_to("t1"));
    src.properties.insert("drop".into(), reference_to("t2"));
    env.add("main", src.clone()).await?;
    let r1 = env.newest_revision("main", "src");

    src.properties.remove("drop");
    src.properties
        .insert("title".into(), PropertyValue::String("y".into()));
    env.put("main", src).await?;
    let r2 = env.newest_revision("main", "src");

    assert!(
        referrers(&env, "t2", None).await?.is_empty(),
        "removed reference"
    );
    assert_eq!(referrers(&env, "t2", Some(&r1)).await?.len(), 1);
    assert_eq!(
        referrers(&env, "t1", None).await?.len(),
        1,
        "kept reference"
    );
    assert_eq!(referrers(&env, "t1", Some(&r1)).await?.len(), 1);

    // The kept reference IS re-put at r2 (references are never skipped: an
    // entry kept below r2 could be masked by a write committed below r2).
    let db = env.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::REFERENCE_INDEX).unwrap();
    let fwd_at = |rev: &raisin_hlc::HLC| {
        raisin_rocksdb::keys::reference_forward_key_versioned(
            TENANT, REPO, "main", WS, "src", "keep", rev, false,
        )
    };
    assert!(db.get_cf(cf, fwd_at(&r1)).unwrap().is_some());
    assert!(db.get_cf(cf, fwd_at(&r2)).unwrap().is_some(), "not re-put");
    Ok(())
}

async fn referrers(
    env: &Env,
    target: &str,
    at: Option<&raisin_hlc::HLC>,
) -> Result<Vec<(String, String)>> {
    env.storage
        .reference_index()
        .find_referencing_nodes_at(env.scope("main"), WS, target, false, at)
        .await
}

/// A write that lands BELOW a version already stored (its transaction took
/// its revision first and staged the node after a later one committed). The
/// later version skipped `a`, keeping it live from the create; the lower
/// write's tombstone of `a = 1` must not mask it at HEAD.
#[tokio::test]
async fn write_below_a_stored_successor_keeps_the_successors_skipped_entries() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("n", "/n", &[("a", "1"), ("b", "1")]))
        .await?;
    env.add("main", node("other", "/other", &[])).await?;

    // tx1 takes its revision now (by writing another node)...
    let tx1 = env.tx("main").await?;
    tx1.put_node(WS, &node("other", "/other", &[("x", "1")]))
        .await?;
    // ...a later transaction updates `n` (b only) and commits first...
    env.put("main", node("n", "/n", &[("a", "1"), ("b", "2")]))
        .await?;
    let r2 = env.newest_revision("main", "n");
    // ...then tx1 writes `n` below it and commits.
    tx1.put_node(WS, &node("n", "/n", &[("a", "2"), ("b", "1")]))
        .await?;
    tx1.commit().await?;
    let r1 = env.newest_revision("main", "other");
    assert!(r1 < r2, "the write must land below the stored successor");

    // HEAD answers as r2 stored it.
    assert_eq!(
        env.indexed("main", "a", "1", None).await?,
        ["n"],
        "a=1 masked"
    );
    assert!(
        env.indexed("main", "a", "2", None).await?.is_empty(),
        "a=2 live"
    );
    assert_eq!(env.indexed("main", "b", "2", None).await?, ["n"]);
    assert!(env.indexed("main", "b", "1", None).await?.is_empty());
    // r1 answers as r1 wrote it.
    assert_eq!(env.indexed("main", "a", "2", Some(&r1)).await?, ["n"]);
    assert!(env.indexed("main", "a", "1", Some(&r1)).await?.is_empty());
    Ok(())
}
