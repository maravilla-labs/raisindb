//! ORDERED_CHILDREN under the delta writer, and the last-label rewrite.

use super::env::{node, Env, REPO, TENANT, WS};
use super::merge_tests::live_labels;
use raisin_error::Result;
use raisin_rocksdb::{cf, keys};
use raisin_storage::{ListOptions, NodeRepository, Storage};

async fn listed(env: &Env) -> Result<Vec<String>> {
    Ok(env
        .storage
        .nodes()
        .list_children(env.scope("main"), "/folder", ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect())
}

fn entries_at(env: &Env, id: &str) -> usize {
    // Every ORDERED_CHILDREN entry (any revision) of `id` under `folder`.
    let prefix = keys::ordered_children_prefix(TENANT, REPO, "main", WS, "folder");
    let db = env.storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    db.iterator_cf(
        cf,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    )
    .map(|item| item.unwrap().0)
    .take_while(|k| k.starts_with(&prefix))
    .filter(|k| k.ends_with(format!("\0{id}").as_bytes()))
    .count()
}

async fn folder_env() -> Result<Env> {
    let env = Env::new(true).await?;
    env.add("main", node("folder", "/folder", &[])).await?;
    for child in ["a", "b", "c"] {
        env.add("main", node(child, &format!("/folder/{child}"), &[]))
            .await?;
    }
    Ok(env)
}

#[tokio::test]
async fn reorder_and_rename_still_rewrite_ordered_entry() -> Result<()> {
    let env = folder_env().await?;
    // A content edit with parent, label and name unchanged still re-puts the
    // entry at its revision (ORDERED_CHILDREN is never skipped: an entry kept
    // below the revision could be masked by a write committed below it).
    let before = entries_at(&env, "a");
    env.put("main", node("a", "/folder/a", &[("title", "x")]))
        .await?;
    assert_eq!(
        entries_at(&env, "a"),
        before + 1,
        "unchanged entry not re-put"
    );

    // A reorder changes the label: written.
    env.storage
        .nodes()
        .move_child_before(env.scope("main"), "/folder", "c", "a", None, None)
        .await?;
    assert_eq!(listed(&env).await?, ["c", "a", "b"]);
    let labels = live_labels(&env, "main", "folder");
    let c = env
        .storage
        .nodes()
        .get(env.scope("main"), "c", None)
        .await?;
    let c_label = &labels.iter().find(|(_, id)| id == "c").unwrap().0;
    assert_eq!(&c.expect("c").order_key, c_label);

    // A rename keeps the label but changes the stored name: written.
    let ctx = env.tx("main").await?;
    let mut b = ctx.get_node(WS, "b").await?.expect("b");
    b.name = "bb".to_string();
    b.path = "/folder/bb".to_string();
    ctx.put_node(WS, &b).await?;
    ctx.commit().await?;
    assert_eq!(listed(&env).await?, ["c", "a", "bb"]);
    let found = env
        .storage
        .nodes()
        .get_by_path(env.scope("main"), "/folder/bb", None)
        .await?;
    assert_eq!(found.map(|n| n.id).as_deref(), Some("b"));
    assert_eq!(live_labels(&env, "main", "folder").len(), 3);
    Ok(())
}

#[tokio::test]
async fn last_order_label_ignores_deleted_child_and_returns_max_live() -> Result<()> {
    let env = folder_env().await?;
    let label_of = |id: &str| {
        live_labels(&env, "main", "folder")
            .into_iter()
            .find(|(_, c)| c == id)
            .map(|(l, _)| l)
            .unwrap()
    };
    let (a, b) = (label_of("a"), label_of("b"));
    let drop_cache = || {
        let db = env.storage.db();
        let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
        db.delete_cf(
            cf,
            keys::last_child_metadata_key(TENANT, REPO, "main", WS, "folder"),
        )
        .unwrap();
    };
    let last = || {
        env.storage
            .nodes_impl()
            .last_order_label_for_append(TENANT, REPO, "main", WS, "folder")
            .unwrap()
    };

    // `c` (the greatest label) moves to the front: its old entry is stale.
    env.storage
        .nodes()
        .move_child_before(env.scope("main"), "/folder", "c", "a", None, None)
        .await?;
    drop_cache();
    assert_eq!(last().as_deref(), Some(b.as_str()), "stale label counted");

    // `b` is deleted: the last LIVE child is `a`.
    let ctx = env.tx("main").await?;
    ctx.delete_node(WS, "b").await?;
    ctx.commit().await?;
    drop_cache();
    assert_eq!(last().as_deref(), Some(a.as_str()), "deleted child counted");

    // The next append lands after every live child.
    env.add("main", node("d", "/folder/d", &[])).await?;
    assert_eq!(listed(&env).await?, ["c", "a", "d"]);
    Ok(())
}
