//! Plan Phase 4: `NodeRepository::get_many_for_read`, the batched snapshot read.
//!
//! The batched read must answer every item EXACTLY as `get` (an id) or
//! `get_by_path` (a path) would at the same revision — tombstones, entries
//! stranded above HEAD, legacy full-`Node` blobs and the Phase 10 path rule
//! included. The reference is `raisin_storage::get_many_by_loop`, one `get`
//! per item, which is also the `sql.batched_fetch = false` fallback.

use crate::node_path_writer_test::{
    folder, head, legacy_tx_put, repo_create, setup, tx_put, BRANCH, REPO, TENANT, WS,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, NodeType};
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::{
    get_many_by_loop, BatchReadItem, BranchScope, CommitMetadata, DeleteNodeOptions,
    NodeRepository, NodeTypeRepository, PropertiesRead, ReadOpts, ReadSnapshot, Storage,
    StorageScope,
};

const VOLATILE: &str = "test:Volatile";

pub(crate) fn branch() -> BranchScope<'static> {
    BranchScope::new(TENANT, REPO, BRANCH)
}

pub(crate) fn id(id: &str) -> BatchReadItem {
    BatchReadItem::id(WS, id)
}

pub(crate) fn path(path: &str) -> BatchReadItem {
    BatchReadItem::path(WS, path)
}

/// The batched read, through `snapshot` when given.
pub(crate) async fn batch(
    storage: &RocksDBStorage,
    items: &[BatchReadItem],
    at: &HLC,
    snapshot: Option<&ReadSnapshot>,
    opts: ReadOpts,
) -> Vec<Option<Node>> {
    storage
        .nodes()
        .get_many_for_read(branch(), items, at, snapshot, opts)
        .await
        .expect("batched read")
}

/// Assert the batched read equals one `get`/`get_by_path` per item — with and
/// without a statement snapshot, loading and skipping properties, with and
/// without `has_children`. Returns the batched answer.
pub(crate) async fn assert_equals_get(
    storage: &RocksDBStorage,
    items: &[BatchReadItem],
    at: &HLC,
    what: &str,
) -> Vec<Option<Node>> {
    let snapshot = storage.nodes().open_read_snapshot();
    assert!(snapshot.is_some(), "RocksDB opens a read snapshot");
    let mut first = None;
    for properties in [PropertiesRead::Load, PropertiesRead::Skip] {
        for has_children in [false, true] {
            let opts = ReadOpts {
                properties: properties.clone(),
                has_children,
            };
            let reference = get_many_by_loop(storage.nodes(), branch(), items, at, &opts)
                .await
                .expect("reference read");
            let pinned = batch(storage, items, at, snapshot.as_ref(), opts.clone()).await;
            let own = batch(storage, items, at, None, opts.clone()).await;
            assert_eq!(pinned, reference, "{what} (snapshot, {opts:?})");
            assert_eq!(own, reference, "{what} (own view, {opts:?})");
            first.get_or_insert(pinned);
        }
    }
    first.unwrap()
}

fn paths(nodes: &[Option<Node>]) -> Vec<Option<String>> {
    nodes
        .iter()
        .map(|n| n.as_ref().map(|n| n.path.clone()))
        .collect()
}

/// Write `value` raw under `id`'s NODES key at `at`.
fn raw_node(storage: &RocksDBStorage, id: &str, at: &HLC, value: &[u8]) {
    let db = storage.db();
    let cf = db.cf_handle(cf::NODES).unwrap();
    db.put_cf(
        cf,
        keys::node_key_versioned(TENANT, REPO, BRANCH, WS, id, at),
        value,
    )
    .unwrap();
}

/// Write `value` raw under `path`'s PATH_INDEX key at `at`.
fn raw_path(storage: &RocksDBStorage, path: &str, at: &HLC, value: &[u8]) {
    let db = storage.db();
    let cf = db.cf_handle(cf::PATH_INDEX).unwrap();
    db.put_cf(
        cf,
        keys::path_index_key_versioned(TENANT, REPO, BRANCH, WS, path, at),
        value,
    )
    .unwrap();
}

/// The newest raw NODES blob of `id` (any revision).
fn newest_blob(storage: &RocksDBStorage, id: &str) -> Vec<u8> {
    let db = storage.db();
    let cf = db.cf_handle(cf::NODES).unwrap();
    let prefix = keys::node_key_prefix(TENANT, REPO, BRANCH, WS, id);
    db.prefix_iterator_cf(cf, &prefix)
        .flatten()
        .find(|(k, _)| k.starts_with(&prefix))
        .map(|(_, v)| v.to_vec())
        .expect("a blob")
}

/// Tombstones — the delete's own, one stranded ABOVE HEAD, merge's legacy
/// `\x00` PATH_INDEX marker — and a legacy full-`Node` blob, each answered as
/// `get` answers it.
#[tokio::test]
async fn batch_get_honours_tombstones_above_head_and_legacy_blobs() -> Result<()> {
    let (storage, _dir) = setup().await?;
    tx_put(&storage, &folder("a", "/a")).await?;
    tx_put(&storage, &folder("b", "/b")).await?;
    tx_put(&storage, &folder("c", "/c")).await?;
    storage
        .nodes()
        .delete(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            "c",
            DeleteNodeOptions::default(),
        )
        .await?;
    legacy_tx_put(&storage, &folder("d", "/d")).await?;
    let h = head(&storage).await?;

    // Above HEAD (stranded writes, a later commit): `a` deleted, `c` alive
    // again at its old path. Invisible at HEAD.
    let above = HLC::new(h.timestamp_ms + 10_000, 0);
    raw_node(&storage, "a", &above, keys::TOMBSTONE_VALUE);
    raw_node(&storage, "c", &above, &newest_blob(&storage, "b"));
    raw_path(&storage, "/c", &above, b"c");
    // Merge's legacy marker AT HEAD: `/b` is vacated (the node `b` itself is
    // still readable by id).
    raw_path(&storage, "/b", &h, b"\x00");

    let items = vec![
        id("a"),
        id("b"),
        id("c"),
        id("d"),
        id("missing"),
        path("/a"),
        path("/b"),
        path("/c"),
        path("/d"),
        path("/nowhere"),
        id("a"), // duplicates answer twice
        BatchReadItem::id("no-such-workspace", "a"),
    ];
    let got = assert_equals_get(&storage, &items, &h, "at HEAD").await;
    assert_eq!(
        paths(&got),
        vec![
            Some("/a".into()),
            Some("/b".into()),
            None,
            Some("/d".into()),
            None,
            Some("/a".into()),
            None,
            None,
            Some("/d".into()),
            None,
            Some("/a".into()),
            None,
        ]
    );
    assert_eq!(got[3].as_ref().unwrap().id, "d", "legacy blob decodes");

    // Read at the revision above HEAD, the stranded entries are what is there.
    let got = assert_equals_get(&storage, &items, &above, "above HEAD").await;
    assert!(got[0].is_none(), "a is deleted above HEAD");
    // `c`'s resurrecting blob above HEAD is a copy of `b`'s record.
    assert!(got[2].is_some(), "c is live above HEAD");
    assert!(got[7].is_some(), "/c names a live node above HEAD");
    Ok(())
}

/// Phase 10's stale-path witness, read in batches: repository create (`/a`
/// at r1), legacy `put_node` rename (`/b` at r2, no NODE_PATH entry). The
/// batch reads `/b` at HEAD and at r2 and `/a` at r1, by id and by path.
#[tokio::test]
async fn repo_create_then_put_node_rename_reads_new_path_at_r2() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let r1 = repo_create(&storage, folder("n1", "/a")).await?;
    let r2 = legacy_tx_put(&storage, &folder("n1", "/b")).await?;
    let r3 = repo_create(&storage, folder("n2", "/c")).await?;
    let r4 = tx_put(&storage, &folder("n2", "/d")).await?;

    let items = vec![
        id("n1"),
        path("/a"),
        path("/b"),
        id("n2"),
        path("/c"),
        path("/d"),
    ];
    let at = |s: &str| Some(s.to_string());
    let cases = [
        (r4, vec![at("/b"), None, at("/b"), at("/d"), None, at("/d")]),
        (r2, vec![at("/b"), None, at("/b"), None, None, None]),
        (r1, vec![at("/a"), at("/a"), None, None, None, None]),
        (r3, vec![at("/b"), None, at("/b"), at("/c"), at("/c"), None]),
    ];
    for (rev, want) in cases {
        let got = assert_equals_get(&storage, &items, &rev, &format!("at {rev}")).await;
        assert_eq!(paths(&got), want, "at {rev}");
    }
    Ok(())
}

/// One snapshot spans every chunk of a statement. A `versionable=false` write
/// rewrites its node IN PLACE at a revision at or below the statement's, so
/// the revision bound cannot hide it: only the pinned view does.
#[tokio::test]
async fn batch_get_snapshot_hides_in_place_overwrite_between_chunks() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let mut volatile: NodeType =
        serde_json::from_value(serde_json::json!({ "name": VOLATILE })).expect("node type literal");
    volatile.id = Some(VOLATILE.to_string());
    volatile.strict = Some(false);
    volatile.allowed_children = vec!["*".to_string()];
    volatile.versionable = Some(false);
    storage
        .node_types()
        .upsert(branch(), volatile, CommitMetadata::system("seed"))
        .await?;

    tx_put(&storage, &folder("x", "/x")).await?;
    let mut y = folder("y", "/y");
    y.node_type = VOLATILE.to_string();
    y.properties
        .insert("title".into(), PropertyValue::String("old".into()));
    tx_put(&storage, &y).await?;
    let h = head(&storage).await?;

    // The statement's view, and its first chunk.
    let snapshot = storage.nodes().open_read_snapshot();
    let first = batch(
        &storage,
        &[id("x")],
        &h,
        snapshot.as_ref(),
        ReadOpts::default(),
    )
    .await;
    assert!(first[0].is_some());

    // Between chunks: the in-place write, at y's own (reused) revision.
    y.properties
        .insert("title".into(), PropertyValue::String("new".into()));
    tx_put(&storage, &y).await?;
    assert_eq!(
        head(&storage).await?,
        h,
        "an in-place write mints no revision"
    );

    let title = |nodes: &[Option<Node>]| {
        nodes[0]
            .as_ref()
            .and_then(|n| n.properties.get("title").cloned())
    };
    let old = Some(PropertyValue::String("old".into()));
    let new = Some(PropertyValue::String("new".into()));
    // The second chunk, through the statement's view: still the old record.
    let pinned = batch(
        &storage,
        &[id("y")],
        &h,
        snapshot.as_ref(),
        ReadOpts::default(),
    )
    .await;
    assert_eq!(title(&pinned), old);
    // Without the view, the same revision bound reads the rewrite.
    let fresh = batch(&storage, &[id("y")], &h, None, ReadOpts::default()).await;
    assert_eq!(title(&fresh), new);
    assert_eq!(
        storage
            .nodes()
            .get(StorageScope::new(TENANT, REPO, BRANCH, WS), "y", Some(&h))
            .await?
            .and_then(|n| n.properties.get("title").cloned()),
        new
    );
    Ok(())
}
