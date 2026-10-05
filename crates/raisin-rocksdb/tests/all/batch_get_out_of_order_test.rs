//! A commit that lands BELOW the statement's revision AFTER its storage view
//! was pinned (Phase 4 review).
//!
//! Transaction A takes revision r1 at its first write, B commits r2 > r1 and
//! moves HEAD, A commits last: HEAD's monotonic guard keeps r2 and A's data
//! lands at r1 — at or below a statement reading at r2. (A replicated peer's
//! ops below the local HEAD do the same.) The statement's live readers (a
//! CHILD_OF page, an index read) return A's rows; a batched read that answered
//! only from the pinned view would pair them with a world without A: a target
//! missing next to the row that names it, an index candidate decoding to a
//! version that no longer matches. The view decides content; a newer live
//! record decides recency.

use crate::batch_get_test::{assert_equals_get, batch, branch, id, path};
use crate::node_path_writer_test::{folder, head, setup, tx_put, BRANCH, REPO, TENANT, WS};
use raisin_core::services::reference_resolver::{ReferenceResolver, ResolveMemo};
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    get_many_by_loop, ListOptions, NodeRepository, ReadOpts, Storage, StorageScope,
};
use serde_json::json;
use std::sync::Arc;

fn reference(id: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: id.to_string(),
        workspace: WS.to_string(),
        path: String::new(),
    })
}

/// A transaction that has written `nodes` — so it holds its revision, taken at
/// its first write — and is NOT committed yet.
async fn uncommitted_tx(
    storage: &RocksDBStorage,
    nodes: &[Node],
) -> Result<Box<dyn TransactionalContext>> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("out of order")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    for node in nodes {
        tx.put_node(WS, node).await?;
    }
    Ok(tx)
}

/// The statement's view is pinned; THEN a commit lands at a revision at or
/// below the statement's — transaction A took r1, B committed r2 > r1 (HEAD),
/// A commits last: HEAD's monotonic guard keeps r2, A's data lands at r1. The
/// statement's live readers (a CHILD_OF page, an index read) already return
/// A's rows, so the batched read must too: a target missing from the view, and
/// a node whose newer version the view lacks, are read live.
#[tokio::test]
async fn batch_get_reads_out_of_order_commit_landed_after_pin() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let mut p = folder("p", "/p");
    p.properties
        .insert("title".into(), PropertyValue::String("old".into()));
    tx_put(&storage, &p).await?;

    // A: create target `t`, point `p` at it — revision r1, uncommitted.
    let mut p_new = p.clone();
    p_new
        .properties
        .insert("title".into(), PropertyValue::String("new".into()));
    p_new.properties.insert("hero".into(), reference("t"));
    let a = uncommitted_tx(&storage, &[folder("t", "/t"), p_new]).await?;
    // B commits after A took its revision: HEAD = r2 > r1.
    let h = tx_put(&storage, &folder("z", "/z")).await?;

    // The statement fixes its revision and pins its view; then A commits.
    let snapshot = storage.nodes().open_read_snapshot();
    a.commit().await?;
    assert_eq!(head(&storage).await?, h, "A landed below HEAD");

    // What the statement's live readers see at h.
    let live_p = storage
        .nodes()
        .get(StorageScope::new(TENANT, REPO, BRANCH, WS), "p", Some(&h))
        .await?
        .expect("p");
    assert!(live_p.properties.contains_key("hero"), "live reads see A");

    let items = vec![id("t"), id("p"), path("/t"), path("/p"), id("z")];
    let got = batch(&storage, &items, &h, snapshot.as_ref(), ReadOpts::default()).await;
    assert_eq!(got[0].as_ref().map(|n| n.path.as_str()), Some("/t"));
    assert_eq!(
        got[1]
            .as_ref()
            .and_then(|n| n.properties.get("title").cloned()),
        Some(PropertyValue::String("new".into())),
        "the candidate decodes to the version the live index named"
    );
    assert!(got[2].is_some(), "a path A created resolves");
    assert_equals_get(&storage, &items, &h, "after the out-of-order commit").await;
    let reference = get_many_by_loop(storage.nodes(), branch(), &items, &h, &ReadOpts::default())
        .await
        .expect("reference read");
    assert_eq!(got, reference, "pinned view == one get per item");
    Ok(())
}

/// The reviewer's scenario through RESOLVE: the statement pins its view, A's
/// out-of-order commit creates target `t` and points child `p` at it, the
/// CHILD_OF page (live) returns `p` naming `t` — and RESOLVE must inline `t`,
/// not leave a bare reference next to the row that names it.
#[tokio::test]
async fn resolve_inlines_target_committed_out_of_order_after_pin() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let storage = Arc::new(storage);
    tx_put(&storage, &folder("p", "/p")).await?;

    let mut p_new = folder("p", "/p");
    p_new.properties.insert("hero".into(), reference("t"));
    let mut t = folder("t", "/t");
    t.properties
        .insert("title".into(), PropertyValue::String("target".into()));
    let a = uncommitted_tx(&storage, &[t, p_new]).await?;
    let h = tx_put(&storage, &folder("z", "/z")).await?;

    // The statement: revision h, its view pinned (RESOLVE's first level).
    let snapshot = storage.nodes().open_read_snapshot();
    let resolver = ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, h)
        .with_memo(Arc::new(ResolveMemo::default()))
        .with_read_snapshot(snapshot.clone());
    a.commit().await?;
    assert_eq!(head(&storage).await?, h, "A landed below HEAD");

    // The listing page (CHILD_OF's direct-children read), live at h, returns
    // A's version of `p`.
    let children = storage
        .nodes()
        .list_root(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            ListOptions {
                max_revision: Some(h),
                ..ListOptions::for_sql()
            },
        )
        .await?;
    let row = children.iter().find(|n| n.id == "p").expect("p listed");
    assert!(row.properties.contains_key("hero"), "the page sees A");

    let doc = json!({ "hero": { "raisin:ref": "t", "raisin:workspace": WS } });
    let out = resolver.resolve_json(WS, &doc, 1, None).await?;
    assert_eq!(out["hero"]["id"], "t", "the target is inlined: {out}");
    assert_eq!(
        out["hero"]["title"], "target",
        "inlined with its properties: {out}"
    );

    // The unbatched resolver (one live `get` per target) agrees.
    let unbatched =
        ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, h).with_batched_fetch(false);
    assert_eq!(unbatched.resolve_json(WS, &doc, 1, None).await?, out);
    Ok(())
}
