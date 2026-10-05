use super::*;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

fn node(props: &[(&str, &str)], updated_micros: i64) -> Node {
    let mut properties = HashMap::new();
    for (k, v) in props {
        properties.insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    properties.insert(
        raisin_models::nodes::RESERVED_SUPERTYPES_KEY.to_string(),
        PropertyValue::Array(vec![PropertyValue::String("base:Thing".into())]),
    );
    Node {
        id: "n1".into(),
        name: "n1".into(),
        path: "/n1".into(),
        node_type: "t:Doc".into(),
        properties,
        updated_at: chrono::DateTime::from_timestamp_micros(updated_micros),
        ..Default::default()
    }
}

fn written(db: &DB) -> Vec<(Vec<u8>, Vec<u8>)> {
    let cf = crate::cf_handle(db, crate::cf::PROPERTY_INDEX).unwrap();
    db.iterator_cf(cf, rocksdb::IteratorMode::Start)
        .map(|r| {
            let (k, v) = r.unwrap();
            (k.to_vec(), v.to_vec())
        })
        .collect()
}

fn write(db: &DB, baseline: Baseline<'_>, new: &Node, rev: HLC, in_place: bool) -> DeltaCounts {
    let target = PropertyIndexTarget::from_db(db).unwrap();
    let ctx = IndexCtx::new("t", "r", "main", "ws");
    let old = match baseline {
        Baseline::Full(old) | Baseline::OutOfOrder { prior: old, .. } => old,
        Baseline::Predecessor(old) => Some(old),
        Baseline::NoPrior => None,
    };
    let targets = in_place
        .then(|| InPlaceTargets::resolve(db, &ctx, old, new, &rev).unwrap())
        .unwrap_or_default();
    let mode = if in_place {
        InPlace::Reused(Some(&targets))
    } else {
        InPlace::No
    };
    let mut batch = WriteBatch::default();
    let counts =
        write_property_index_delta(&mut batch, target, &ctx, baseline, new, &rev, mode).unwrap();
    db.write(batch).unwrap();
    counts
}

fn open() -> (tempfile::TempDir, DB) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(dir.path()).unwrap();
    (dir, db)
}

#[test]
fn entries_include_membership_and_pseudo_properties() {
    let names: Vec<String> = entries_of(&node(&[("slug", "a")], 1))
        .into_iter()
        .map(|e| e.name)
        .collect();
    for expected in [
        "slug",
        "__node_type",
        "__name",
        "__updated_at",
        raisin_models::nodes::INDEXED_SUPERTYPE_KEY,
    ] {
        assert!(names.iter().any(|n| n == expected), "missing {expected}");
    }
}

#[test]
fn predecessor_writes_only_what_changed() {
    let (_dir, db) = open();
    let old = node(&[("slug", "a"), ("title", "x")], 1);
    let created = write(&db, Baseline::NoPrior, &old, HLC::new(10, 0), false);
    assert_eq!(created.tombstones, 0);
    let new = node(&[("slug", "a"), ("title", "y")], 2);
    let counts = write(
        &db,
        Baseline::Predecessor(&old),
        &new,
        HLC::new(20, 0),
        false,
    );
    // title x->y and updated_at 1->2: two tombstones, two puts.
    assert_eq!(counts.tombstones, 2, "{counts:?}");
    assert_eq!(counts.puts, 2, "{counts:?}");
    assert_eq!(counts.skipped, created.puts - 2);
    let full = write(
        &db,
        Baseline::Full(Some(&old)),
        &new,
        HLC::new(30, 0),
        false,
    );
    assert_eq!(full.puts, created.puts);
    assert_eq!(full.skipped, 0);
}

#[test]
fn in_place_tombstone_lands_on_the_newest_group_entry_above_the_revision() {
    let (_dir, db) = open();
    let old = node(&[("title", "x")], 1);
    write(&db, Baseline::NoPrior, &old, HLC::new(10, 0), false);
    // A rebuild re-wrote the node's entries at HEAD, above its revision.
    write(&db, Baseline::Full(None), &old, HLC::new(50, 0), false);
    let new = node(&[("title", "y")], 2);
    write(&db, Baseline::Full(Some(&old)), &new, HLC::new(10, 0), true);
    // The `title = x` group: newest entry must be a tombstone.
    let ctx = IndexCtx::new("t", "r", "main", "ws");
    let prefix = PropertyEntry {
        published: false,
        name: "title".into(),
        value: EntryValue::Text("x".into()),
    }
    .value_prefix(&ctx);
    let first = written(&db)
        .into_iter()
        .find(|(k, _)| k.starts_with(&prefix))
        .unwrap();
    assert_eq!(first.1, b"T", "stale value must be masked at HEAD");
}

fn other(id: &str, title: &str) -> Node {
    Node {
        id: id.into(),
        ..node(&[("title", title)], 1)
    }
}

/// The in-place above-R lookup walks the value's entries of ALL nodes; for a
/// common value it is capped, and then lands at R instead of scanning on.
#[test]
fn in_place_group_walk_is_capped() {
    let (_dir, db) = open();
    let ctx = IndexCtx::new("t", "r", "main", "ws");
    let mine = node(&[("title", "x")], 1);
    // This node's group entry above R...
    write(&db, Baseline::NoPrior, &mine, HLC::new(60, 0), false);
    let title = PropertyEntry {
        published: false,
        name: "title".into(),
        value: EntryValue::Text("x".into()),
    };
    let found = InPlaceTargets::resolve(&db, &ctx, None, &mine, &HLC::new(10, 0)).unwrap();
    assert_eq!(found.get(&title), Some(HLC::new(60, 0)));

    // ...buried under more newer entries of the same value than the cap.
    for i in 0..(in_place::MAX_KEYS_PER_GROUP + 10) {
        write(
            &db,
            Baseline::NoPrior,
            &other(&format!("o{i:04}"), "x"),
            HLC::new(70, 0),
            false,
        );
    }
    let capped_before = in_place_scans_capped();
    let found = InPlaceTargets::resolve(&db, &ctx, None, &mine, &HLC::new(10, 0)).unwrap();
    assert_eq!(found.get(&title), None, "walk not capped");
    assert!(in_place_scans_capped() > capped_before);
}
