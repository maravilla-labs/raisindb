//! The Phase 10 path read rule: the newer, by revision, of `NODE_PATH` and a
//! legacy full-`Node` blob's embedded path; a disagreeing tie goes to the path
//! `PATH_INDEX` confirms at that revision.

use super::{
    current_path, decode_node_blob, deserialize_node_with_path, embedded_path_may_win,
    embedded_path_of, NodeScope,
};
use crate::{cf, keys, StorageNode};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{Options, DB};

const T: &str = "t";
const R: &str = "r";
const B: &str = "main";
const WS: &str = "ws";
const ID: &str = "n1";

fn open() -> (DB, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    (
        DB::open_cf(
            &opts,
            dir.path(),
            [cf::NODE_PATH, cf::NODES, cf::PATH_INDEX],
        )
        .unwrap(),
        dir,
    )
}

fn rev(ms: u64) -> HLC {
    HLC::new(ms, 0)
}

fn put_node_path(db: &DB, at: &HLC, path: &str) {
    let key = keys::node_path_key_versioned(T, R, B, WS, ID, at);
    db.put_cf(db.cf_handle(cf::NODE_PATH).unwrap(), key, path.as_bytes())
        .unwrap();
}

fn node(path: &str) -> Node {
    Node {
        id: ID.to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Node::default()
    }
}

/// What the pre-Phase-10 transaction writer stored: the full Node, named.
fn legacy_blob(path: &str) -> Vec<u8> {
    rmp_serde::to_vec_named(&node(path)).unwrap()
}

/// What the repository (and the Phase 10 transaction writer) stores.
fn storage_blob(path: &str) -> Vec<u8> {
    rmp_serde::to_vec_named(&StorageNode::from_node(&node(path), None)).unwrap()
}

fn read(db: &DB, blob: &[u8], read_at: &HLC, blob_at: &HLC) -> String {
    deserialize_node_with_path(db, blob, T, R, B, WS, ID, read_at, blob_at)
        .unwrap()
        .path
}

fn put_path_index(db: &DB, at: &HLC, path: &str, value: &[u8]) {
    let key = keys::path_index_key_versioned(T, R, B, WS, path, at);
    db.put_cf(db.cf_handle(cf::PATH_INDEX).unwrap(), key, value)
        .unwrap();
}

fn put_blob(db: &DB, at: &HLC, blob: &[u8]) {
    let key = keys::node_key_versioned(T, R, B, WS, ID, at);
    db.put_cf(db.cf_handle(cf::NODES).unwrap(), key, blob)
        .unwrap();
}

const SCOPE: NodeScope<'static> = NodeScope {
    tenant_id: T,
    repo_id: R,
    branch: B,
    workspace: WS,
    node_id: ID,
};

/// Both readers of the rule — the blob decoder and the blob-less resolver.
fn both(db: &DB, blob: &[u8], at: &HLC) -> (String, Option<String>) {
    (
        read(db, blob, at, at),
        current_path(db, SCOPE, Some(at))
            .unwrap()
            .and_then(|c| c.path),
    )
}

#[test]
fn only_a_blob_not_older_than_the_entry_may_win() {
    assert!(embedded_path_may_win(None, &rev(5)));
    assert!(embedded_path_may_win(Some(&rev(4)), &rev(5)));
    assert!(embedded_path_may_win(Some(&rev(5)), &rev(5)));
    assert!(!embedded_path_may_win(Some(&rev(6)), &rev(5)));
}

/// `put_node(c)` then an ancestor move in ONE transaction, as a pre-Phase-10
/// binary stored it: legacy blob /a/c at R, the move's NODE_PATH /x/a/c at R,
/// PATH_INDEX moved at R. A tie that went to the blob read the stale /a/c.
#[test]
fn put_then_ancestor_move_in_one_tx_tie_reads_the_moved_path() {
    let (db, _dir) = open();
    let r = rev(5);
    let blob = legacy_blob("/a/c");
    put_blob(&db, &r, &blob);
    put_node_path(&db, &r, "/x/a/c");
    put_path_index(&db, &r, "/a/c", b"T");
    put_path_index(&db, &r, "/x/a/c", ID.as_bytes());
    let (decoded, resolved) = both(&db, &blob, &r);
    assert_eq!(decoded, "/x/a/c");
    assert_eq!(resolved.as_deref(), Some("/x/a/c"));
}

/// A `versionable=false` node renamed IN PLACE through the legacy `put_node`
/// at its reused revision, over a repository-written NODE_PATH at that same
/// revision: the blob holds the new path, and PATH_INDEX confirms it.
#[test]
fn versionable_false_in_place_rename_tie_reads_the_blob_path() {
    let (db, _dir) = open();
    let r = rev(5);
    let blob = legacy_blob("/renamed");
    put_blob(&db, &r, &blob);
    put_node_path(&db, &r, "/original");
    put_path_index(&db, &r, "/original", b"T");
    put_path_index(&db, &r, "/renamed", ID.as_bytes());
    let (decoded, resolved) = both(&db, &blob, &r);
    assert_eq!(decoded, "/renamed");
    assert_eq!(resolved.as_deref(), Some("/renamed"));
}

/// Deleted and recreated through the legacy `put_node` in one transaction:
/// the NODE_PATH tombstone at R loses to the recreated blob PATH_INDEX names.
#[test]
fn recreate_after_delete_in_one_tx_tie_reads_the_blob_path() {
    let (db, _dir) = open();
    let r = rev(5);
    let blob = legacy_blob("/back");
    put_blob(&db, &r, &blob);
    put_node_path(&db, &r, "T");
    put_path_index(&db, &r, "/back", ID.as_bytes());
    let (decoded, resolved) = both(&db, &blob, &r);
    assert_eq!(decoded, "/back");
    assert_eq!(resolved.as_deref(), Some("/back"));
}

/// Repository create (`NODE_PATH` /a at r1), then a pre-Phase-10 `put_node`
/// rename (full blob /b at r2, no entry): /b, not the stale /a.
#[test]
fn a_newer_embedded_path_beats_an_older_entry() {
    let (db, _dir) = open();
    put_node_path(&db, &rev(1), "/a");
    let blob = legacy_blob("/b");
    assert_eq!(read(&db, &blob, &rev(2), &rev(2)), "/b");
    assert_eq!(read(&db, &blob, &rev(9), &rev(2)), "/b");
}

/// A later ancestor move writes `NODE_PATH` above the blob: the entry wins.
#[test]
fn a_newer_entry_beats_an_older_embedded_path() {
    let (db, _dir) = open();
    put_node_path(&db, &rev(3), "/moved/b");
    let blob = legacy_blob("/b");
    assert_eq!(read(&db, &blob, &rev(3), &rev(2)), "/moved/b");
    // ...but not for a read before the move.
    assert_eq!(read(&db, &blob, &rev(2), &rev(2)), "/b");
}

#[test]
fn a_storage_blob_reads_its_path_from_the_index() {
    let (db, _dir) = open();
    put_node_path(&db, &rev(1), "/a");
    put_node_path(&db, &rev(2), "/b");
    let blob = storage_blob("/ignored");
    assert_eq!(read(&db, &blob, &rev(1), &rev(1)), "/a");
    assert_eq!(read(&db, &blob, &rev(2), &rev(2)), "/b");
}

/// A legacy full-Node blob round-trips: the embedded path survives decoding
/// it as a `StorageNode`, through every decoder that looks at it.
#[test]
fn legacy_full_node_blob_round_trips() {
    let (db, _dir) = open();
    let blob = legacy_blob("/x/y");
    let as_storage: StorageNode = rmp_serde::from_slice(&blob).unwrap();
    assert_eq!(as_storage.embedded_path(), Some("/x/y"));
    assert_eq!(embedded_path_of(&blob).as_deref(), Some("/x/y"));
    assert_eq!(decode_node_blob(&blob).unwrap().0.path, "/x/y");
    assert_eq!(read(&db, &blob, &rev(1), &rev(1)), "/x/y");

    // A StorageNode blob embeds nothing, and never writes the field.
    let blob = storage_blob("/x/y");
    assert_eq!(embedded_path_of(&blob), None);
    assert!(!String::from_utf8_lossy(&blob).contains("/x/y"));
}
