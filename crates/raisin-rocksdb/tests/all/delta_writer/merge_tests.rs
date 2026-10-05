//! Merge never skips; promotion diffs against `old_dst`; `versionable=false`.

use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type;
use raisin_context::ResolutionType;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::{cf, keys};
use raisin_storage::{BranchRepository, ListOptions, NodeRepository, Storage};
use std::collections::HashMap;

/// The source changed `slug` A -> B; the target kept A, so its live `slug = A`
/// entry sits at the CREATE revision (skipped by the target's own edit). Merge
/// replays the source's tombstone of A at its original revision; a resolution
/// that skipped too would leave A masked and the node gone from `slug = A`.
#[tokio::test]
async fn merge_keep_ours_after_skip_unchanged_property_still_matches() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("c", "/c", &[("slug", "A"), ("title", "base")]))
        .await?;
    env.fork("feature").await?;
    env.rebuild("feature").await?;
    env.put("main", node("c", "/c", &[("slug", "A"), ("title", "ours")]))
        .await?;
    env.put(
        "feature",
        node("c", "/c", &[("slug", "B"), ("title", "theirs")]),
    )
    .await?;
    env.conflict_and_resolve("feature", "c", ResolutionType::KeepOurs)
        .await?;
    assert_eq!(env.indexed("main", "slug", "A", None).await?, ["c"]);
    assert!(env.indexed("main", "slug", "B", None).await?.is_empty());
    assert_eq!(env.indexed("main", "title", "ours", None).await?, ["c"]);
    assert!(env
        .indexed("main", "title", "theirs", None)
        .await?
        .is_empty());
    Ok(())
}

/// [`crate::merge_apply_funnel_test::merge_keep_ours_source_only_value_not_matched`]
/// with skip-unchanged on.
#[tokio::test]
async fn merge_keep_ours_source_only_value_not_matched() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("c", "/c", &[("title", "base")]))
        .await?;
    env.fork("feature").await?;
    env.rebuild("feature").await?;
    env.put("main", node("c", "/c", &[("title", "ours")]))
        .await?;
    env.put(
        "feature",
        node("c", "/c", &[("title", "theirs"), ("extra", "src-only")]),
    )
    .await?;
    env.conflict_and_resolve("feature", "c", ResolutionType::KeepOurs)
        .await?;
    assert!(env
        .indexed("main", "extra", "src-only", None)
        .await?
        .is_empty());
    assert!(env
        .indexed("main", "title", "theirs", None)
        .await?
        .is_empty());
    assert_eq!(env.indexed("main", "title", "ours", None).await?, ["c"]);
    Ok(())
}

/// Live `(label, child)` pairs under `parent_id` on `branch`, raw.
pub(super) fn live_labels(env: &Env, branch: &str, parent_id: &str) -> Vec<(String, String)> {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, branch, WS, parent_id);
    let db = env.storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    let mut decided = std::collections::HashSet::new();
    let mut live = Vec::new();
    let iter = db.iterator_cf(
        cf,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    );
    for item in iter {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        let suffix = &key[prefix.len()..];
        let Some(label_end) = suffix.iter().position(|b| *b == 0) else {
            continue;
        };
        let child_start = label_end + 1 + 16 + 1;
        if suffix.len() <= child_start {
            continue;
        }
        let label = String::from_utf8_lossy(&suffix[..label_end]).into_owned();
        let child = String::from_utf8_lossy(&suffix[child_start..]).into_owned();
        if decided.insert((label.clone(), child.clone())) && !keys::is_tombstone_value(&value) {
            live.push((label, child));
        }
    }
    live
}

/// The source reorders `c` to the front; keep-ours keeps the target's label,
/// even though the target's own edit did not re-put its ORDERED_CHILDREN
/// entry (skipped: parent, label and name unchanged).
#[tokio::test]
async fn merge_keep_ours_preserves_target_order_label() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("folder", "/folder", &[])).await?;
    env.add("main", node("a", "/folder/a", &[])).await?;
    env.add("main", node("c", "/folder/c", &[("title", "base")]))
        .await?;
    let before: HashMap<String, String> = live_labels(&env, "main", "folder")
        .into_iter()
        .map(|(l, c)| (c, l))
        .collect();
    env.fork("feature").await?;
    env.rebuild("feature").await?;

    env.put("main", node("c", "/folder/c", &[("title", "ours")]))
        .await?;
    env.storage
        .nodes()
        .move_child_before(env.scope("feature"), "/folder", "c", "a", None, None)
        .await?;
    env.put("feature", node("c", "/folder/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("feature", "c", ResolutionType::KeepOurs)
        .await?;

    let after = live_labels(&env, "main", "folder");
    let c_labels: Vec<&String> = after
        .iter()
        .filter(|(_, c)| c == "c")
        .map(|(l, _)| l)
        .collect();
    assert_eq!(c_labels, vec![&before["c"]], "target label kept: {after:?}");
    let listed: Vec<String> = env
        .storage
        .nodes()
        .list_children(env.scope("main"), "/folder", ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(listed, ["a", "c"]);
    let c = env
        .storage
        .nodes()
        .get(env.scope("main"), "c", None)
        .await?;
    assert_eq!(c.expect("c").order_key, before["c"]);
    Ok(())
}

#[tokio::test]
async fn promote_removed_property_not_matched_on_publish_branch() -> Result<()> {
    let env = Env::new(true).await?;
    env.storage
        .branches()
        .create_branch(TENANT, REPO, "publish", "system", None, None, false, false)
        .await?;
    env.rebuild("publish").await?;
    env.add(
        "main",
        node("p", "/p", &[("campaign", "summer"), ("title", "x")]),
    )
    .await?;
    let roots = vec!["/p".to_string()];
    let promote = || async {
        env.storage
            .nodes()
            .copy_nodes_across_branches(
                TENANT, REPO, "main", "publish", WS, &roots, true, false, None, None,
            )
            .await
    };
    promote().await?;
    assert_eq!(
        env.indexed("publish", "campaign", "summer", None).await?,
        ["p"]
    );
    assert_eq!(env.indexed("publish", "title", "x", None).await?, ["p"]);

    env.put("main", node("p", "/p", &[("title", "x")])).await?;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    promote().await?;
    assert!(
        env.indexed("publish", "campaign", "summer", None)
            .await?
            .is_empty(),
        "a property removed on the source still matches on the publish branch"
    );
    assert_eq!(env.indexed("publish", "title", "x", None).await?, ["p"]);
    Ok(())
}

fn ping(title: &str) -> raisin_models::nodes::Node {
    let mut n = node("ping", "/ping", &[("title", title)]);
    n.node_type = "test:Ping".to_string();
    n
}

/// An in-place (`versionable=false`) update is a full put at the reused
/// revision R — and a tombstone lands on the newest entry of its group when
/// one sits ABOVE R (here: one a pre-Phase-7 rebuild wrote at HEAD).
#[tokio::test]
async fn versionable_false_update_with_skip() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Ping", None, Some(false)).await?;
    env.add("main", ping("one")).await?;
    let r = env.newest_revision("main", "ping");
    // HEAD moves past R, and a legacy rebuild re-put `title = one` there.
    env.add("main", node("other", "/other", &[])).await?;
    let head = env.newest_revision("main", "other");
    assert!(head > r);
    let db = env.storage.db();
    let cf = db.cf_handle(cf::PROPERTY_INDEX).unwrap();
    let key = keys::property_index_key_versioned(
        TENANT, REPO, "main", WS, "title", "one", &head, "ping", false,
    );
    db.put_cf(cf, key, b"ping").unwrap();
    assert_eq!(env.indexed("main", "title", "one", None).await?, ["ping"]);

    env.put("main", ping("two")).await?;
    assert_eq!(env.newest_revision("main", "ping"), r, "written in place");
    assert_eq!(env.indexed("main", "title", "two", None).await?, ["ping"]);
    assert!(
        env.indexed("main", "title", "one", None).await?.is_empty(),
        "the in-place tombstone must mask the entry above R"
    );
    Ok(())
}

/// `versionable=false` writes mint no revision, so merge conflict detection
/// does not see them (documented); both branches' writes land on the same R0
/// keys, and after a merge main's in-place value still answers.
#[tokio::test]
async fn versionable_false_with_merge() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Ping", None, Some(false)).await?;
    env.add("main", ping("one")).await?;
    env.add("main", node("doc", "/doc", &[("title", "base")]))
        .await?;
    env.fork("feature").await?;
    env.rebuild("feature").await?;
    env.put("main", ping("two")).await?;
    env.put("feature", node("doc", "/doc", &[("title", "feature")]))
        .await?;
    let result = env
        .storage
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
        .await?;
    assert!(result.conflicts.is_empty(), "{:?}", result.conflicts);
    assert_eq!(
        env.indexed("main", "title", "feature", None).await?,
        ["doc"]
    );
    assert_eq!(env.indexed("main", "title", "two", None).await?, ["ping"]);
    assert!(env.indexed("main", "title", "one", None).await?.is_empty());
    env.put("main", ping("three")).await?;
    assert_eq!(env.indexed("main", "title", "three", None).await?, ["ping"]);
    assert!(env.indexed("main", "title", "two", None).await?.is_empty());
    let n = env
        .storage
        .nodes()
        .get(env.scope("main"), "ping", None)
        .await?;
    assert_eq!(
        n.expect("ping").properties.get("title"),
        Some(&PropertyValue::String("three".into()))
    );
    Ok(())
}
