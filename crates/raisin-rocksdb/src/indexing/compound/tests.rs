//! Unit tests of the one compound writer, delete tombstoner and key grammar.

use super::*;
use crate::indexing::{Baseline, IndexCtx};
use crate::repositories::compound_index::newest_per_tuple;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::collections::HashMap;
use std::sync::Arc;

pub(super) const CTX: IndexCtx<'static> = IndexCtx {
    tenant_id: "t",
    repo_id: "r",
    branch: "main",
    workspace: "ws",
};

pub(super) fn open() -> (tempfile::TempDir, DB) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(dir.path()).unwrap();
    (dir, db)
}

/// `t:Doc` with `by_status (status String, priority Integer)`.
pub(super) fn defs() -> DefsSet {
    let def = serde_json::from_value(serde_json::json!({
        "name": "by_status",
        "columns": [
            { "property": "status", "column_type": "String" },
            { "property": "priority", "column_type": "Integer" }
        ],
        "has_order_column": true
    }))
    .unwrap();
    DefsSet::from_pairs([(
        "t:Doc".to_string(),
        Arc::new(TypeIndexDefs {
            compound: vec![def],
            unique: vec![],
        }),
    )])
}

pub(super) fn doc(status: &str, priority: i64) -> Node {
    Node {
        id: "n1".into(),
        name: "n1".into(),
        path: "/n1".into(),
        node_type: "t:Doc".into(),
        properties: HashMap::from([
            ("status".to_string(), PropertyValue::String(status.into())),
            ("priority".to_string(), PropertyValue::Integer(priority)),
        ]),
        ..Default::default()
    }
}

pub(super) fn write(db: &DB, baseline: Baseline<'_>, new: &Node, rev: HLC) -> CompoundCounts {
    let mut batch = WriteBatch::default();
    let counts = write_compound_delta(&mut batch, db, &CTX, &defs(), baseline, new, &rev).unwrap();
    db.write(batch).unwrap();
    counts
}

/// The node ids the reader returns for `status = s`, as of `at`.
pub(super) fn read(db: &DB, status: &str, at: Option<HLC>) -> Vec<String> {
    let cf = crate::cf_handle(db, crate::cf::COMPOUND_INDEX).unwrap();
    let prefix = crate::keys::compound_index_prefix(
        "t",
        "r",
        "main",
        "ws",
        "by_status",
        &[raisin_storage::CompoundColumnValue::String(status.into())],
        false,
    );
    let entries: Vec<(Vec<u8>, bool)> = crate::prefix_scan(db, cf, &prefix)
        .map(|item| {
            let (k, v) = item.unwrap();
            (k.to_vec(), crate::keys::is_tombstone_value(&v))
        })
        .take_while(|(k, _)| k.starts_with(&prefix))
        .collect();
    newest_per_tuple(
        entries.iter().map(|(k, dead)| (k.as_slice(), *dead)),
        at.as_ref(),
        None,
    )
    .into_iter()
    .map(|e| e.node_id)
    .collect()
}

#[test]
fn entry_key_round_trips_through_null_bytes() {
    // Priority 0 and a zero HLC counter both put `\0` bytes inside the key.
    let group = compound_entries(defs().compound("t:Doc"), &CTX, &doc("open", 0))
        .into_iter()
        .next()
        .unwrap();
    let rev = HLC::new(5, 0);
    let key = entry_key(&group, &rev, "n1");
    let (g, at, id) = parse_entry_key(&key).unwrap();
    assert_eq!((g, at, id), (group.as_slice(), rev, "n1"));
}

#[test]
fn changed_tuple_is_tombstoned_at_the_revision_and_history_reads_it() {
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    let v2 = doc("done", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    let counts = write(&db, Baseline::Predecessor(&v1), &v2, HLC::new(20, 0));
    assert_eq!((counts.puts, counts.tombstones), (1, 1));
    assert_eq!(read(&db, "open", Some(HLC::new(15, 0))), vec!["n1"]);
    assert!(read(&db, "done", Some(HLC::new(15, 0))).is_empty());
    assert!(read(&db, "open", None).is_empty());
    assert_eq!(read(&db, "done", None), vec!["n1"]);
}

#[test]
fn unchanged_tuple_is_skipped_only_under_a_predecessor() {
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    let mut v2 = v1.clone();
    v2.properties
        .insert("title".into(), PropertyValue::String("x".into()));
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    let skipped = write(&db, Baseline::Predecessor(&v1), &v2, HLC::new(20, 0));
    assert_eq!((skipped.puts, skipped.skipped), (0, 1));
    let full = write(&db, Baseline::Full(Some(&v2)), &v2, HLC::new(30, 0));
    assert_eq!((full.puts, full.skipped, full.tombstones), (1, 0, 0));
    assert_eq!(read(&db, "open", None), vec!["n1"]);
}

#[test]
fn tombstone_lands_on_an_entry_above_the_write_revision() {
    // A pre-Phase-8 rebuild stamped the entry with the branch HEAD (100); an
    // in-place write at the node's own revision (10) that changes the tuple
    // must end THAT entry, or `open` matches forever.
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(100, 0));
    write(
        &db,
        Baseline::Full(Some(&v1)),
        &doc("done", 1),
        HLC::new(10, 0),
    );
    assert!(read(&db, "open", None).is_empty());
    assert_eq!(read(&db, "done", None), vec!["n1"]);
}

#[test]
fn out_of_order_write_reasserts_a_skip_written_successor() {
    // v1@10 open; v3@30 written with skip (open unchanged, nothing put at 30);
    // then v2@20 arrives (done): its `open` tombstone at 20 must not hide v3.
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    let v3 = doc("open", 1);
    write(&db, Baseline::Predecessor(&v1), &v3, HLC::new(30, 0));
    let successors = vec![(HLC::new(30, 0), Some(v3))];
    write(
        &db,
        Baseline::OutOfOrder {
            prior: Some(&v1),
            successors: &successors,
        },
        &doc("done", 1),
        HLC::new(20, 0),
    );
    assert_eq!(read(&db, "open", None), vec!["n1"]);
    assert!(read(&db, "done", None).is_empty());
    assert_eq!(read(&db, "done", Some(HLC::new(25, 0))), vec!["n1"]);
}

#[test]
fn a_tombstone_hides_only_its_own_tuple() {
    // Old reader: one node-wide "tombstoned" set, so ending the `a` tuple hid
    // the node's live `b` tuple sorting after it.
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    let v2 = doc("open", 2);
    write(&db, Baseline::Predecessor(&v1), &v2, HLC::new(20, 0));
    assert_eq!(read(&db, "open", None), vec!["n1"]);
    assert_eq!(read(&db, "open", Some(HLC::new(15, 0))), vec!["n1"]);
}

#[test]
fn delete_derives_with_warm_definitions_and_scans_when_cold() {
    let (_dir, db) = open();
    let v1 = doc("open", 1);
    write(&db, Baseline::NoPrior, &v1, HLC::new(10, 0));
    // Cold (nothing cached for this database): the scan fallback.
    let mut batch = WriteBatch::default();
    tombstone_compound_for_delete(&mut batch, &db, &CTX, &v1, &HLC::new(20, 0)).unwrap();
    db.write(batch).unwrap();
    assert!(read(&db, "open", None).is_empty());
    assert_eq!(read(&db, "open", Some(HLC::new(15, 0))), vec!["n1"]);
}

/// A date-like string comes back from storage as a `Date` (untagged
/// `PropertyValue`): both spellings must derive ONE entry, or an update
/// cannot tombstone what the insert wrote and a rebuild drops the node.
#[test]
fn date_like_string_and_its_stored_date_derive_one_entry() {
    let def: raisin_models::nodes::properties::schema::CompoundIndexDefinition =
        serde_json::from_value(serde_json::json!({
            "name": "by_expiry",
            "columns": [{ "property": "expires", "column_type": "String" }],
            "has_order_column": false
        }))
        .unwrap();
    let with = |value: PropertyValue| Node {
        id: "n1".into(),
        node_type: "t:Doc".into(),
        properties: HashMap::from([("expires".to_string(), value)]),
        ..Default::default()
    };
    let text = with(PropertyValue::String("2026-01-01T00:00:00.000Z".into()));
    let stored: Node = {
        let bytes = rmp_serde::to_vec_named(&text).unwrap();
        rmp_serde::from_slice(&bytes).unwrap()
    };
    assert!(
        matches!(stored.properties["expires"], PropertyValue::Date(_)),
        "the premise: storage turns the string into a date"
    );
    let defs = std::slice::from_ref(&def);
    let a = compound_entries(defs, &CTX, &text);
    assert_eq!(a.len(), 1);
    assert_eq!(a, compound_entries(defs, &CTX, &stored));
    // The SQL executor encodes the literal the same way.
    assert_eq!(
        raisin_storage::CompoundColumnValue::text("2026-01-01T00:00:00.000Z"),
        raisin_storage::CompoundColumnValue::String("2026-01-01T00:00:00Z".into())
    );
}
