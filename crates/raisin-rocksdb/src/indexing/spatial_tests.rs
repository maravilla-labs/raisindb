//! Key-level tests for nested geometry indexing.
//!
//! These assert on the **key set** a write produces rather than on query results,
//! because the key set is where the writer and the tombstoner have to agree. A
//! query-level test passes for a while even when the two formats have drifted;
//! it only fails once a stale entry happens to be inside a scanned cell.

use super::spatial::{write_node_spatial_indexes, SpatialIndexTargets, SPATIAL_TOMBSTONE};
use super::spatial_tombstone::tombstone_superseded_spatial_indexes;
use super::{IndexCtx, NodeSpatialPolicies};
use crate::{cf, cf_handle, keys};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::{GeoJson, PropertyValue, SpatialPolicy};
use raisin_models::nodes::Node;
use rocksdb::{Options, WriteBatch, DB};
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;

fn test_db() -> (TempDir, DB) {
    let dir = TempDir::new().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = DB::open_cf(&opts, dir.path(), vec![cf::SPATIAL_INDEX]).unwrap();
    (dir, db)
}

fn point(lon: f64, lat: f64) -> PropertyValue {
    PropertyValue::Geometry(GeoJson::point(lon, lat))
}

fn element(field: &str, value: PropertyValue) -> PropertyValue {
    let mut content = HashMap::new();
    content.insert(field.to_string(), value);
    PropertyValue::Element(Element {
        uuid: String::new(),
        element_type: "demo:Section".to_string(),
        content,
    })
}

fn node_with(properties: HashMap<String, PropertyValue>) -> Node {
    let mut node = Node::default();
    node.id = "n1".to_string();
    node.node_type = "demo:Place".to_string();
    node.properties = properties;
    node
}

/// Every (property_path, is_live) pair the batch produced.
///
/// Values distinguish a live entry from a tombstone, which is the whole point:
/// "the path is present" is not the same claim as "the path still matches".
fn commit_and_read(db: &DB, batch: WriteBatch) -> Vec<(String, bool)> {
    db.write(batch).unwrap();
    let cf = cf_handle(db, cf::SPATIAL_INDEX).unwrap();
    let mut out = Vec::new();
    for item in db.iterator_cf(cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item.unwrap();
        let parsed = keys::parse_spatial_index_key(&key).expect("key must parse");
        out.push((
            parsed.property_name.to_string(),
            value.as_ref() != SPATIAL_TOMBSTONE,
        ));
    }
    out
}

fn live_paths(entries: &[(String, bool)]) -> Vec<String> {
    let mut paths: Vec<String> = entries
        .iter()
        .filter(|(_, live)| *live)
        .map(|(p, _)| p.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    paths.sort();
    paths
}

fn tombstoned_paths(entries: &[(String, bool)]) -> Vec<String> {
    let mut paths: Vec<String> = entries
        .iter()
        .filter(|(_, live)| !*live)
        .map(|(p, _)| p.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    paths.sort();
    paths
}

fn write(db: &DB, node: &Node, revision: &HLC) -> Vec<(String, bool)> {
    let targets = SpatialIndexTargets::from_db(db).unwrap();
    let ctx = IndexCtx::new("t", "r", "main", "ws");
    let mut batch = WriteBatch::default();
    write_node_spatial_indexes(
        &mut batch,
        &targets,
        &ctx,
        node,
        revision,
        &NodeSpatialPolicies::all_default(),
    )
    .unwrap();
    commit_and_read(db, batch)
}

/// The key set ONE update (or delete, with `new == None`) produces.
///
/// Deliberately committed into a fresh database rather than on top of the
/// previous revision's entries: both the writer and the tombstoner derive their
/// keys from the geometries alone and perform ZERO reads, so the batch is
/// independent of what is already stored — and reading a fresh database is what
/// makes "which paths did THIS update touch" an exact assertion instead of a
/// difference against a growing set.
fn update(old: &Node, new: Option<&Node>, revision: &HLC) -> Vec<(String, bool)> {
    let (_dir, db) = test_db();
    let targets = SpatialIndexTargets::from_db(&db).unwrap();
    let ctx = IndexCtx::new("t", "r", "main", "ws");
    let policies = NodeSpatialPolicies::all_default();
    let mut batch = WriteBatch::default();
    tombstone_superseded_spatial_indexes(&mut batch, &targets, &ctx, old, new, revision, &policies)
        .unwrap();
    if let Some(new) = new {
        write_node_spatial_indexes(&mut batch, &targets, &ctx, new, revision, &policies).unwrap();
    }
    commit_and_read(&db, batch)
}

#[test]
fn top_level_geometry_indexes_under_the_bare_property_name() {
    let (_dir, db) = test_db();
    let node = node_with(HashMap::from([(
        "location".to_string(),
        point(8.54, 47.37),
    )]));
    let entries = write(&db, &node, &HLC::new(100, 1));
    assert_eq!(live_paths(&entries), vec!["location".to_string()]);
}

#[test]
fn geometry_inside_an_object_indexes_under_the_dotted_path() {
    let (_dir, db) = test_db();
    let venue = HashMap::from([("geo".to_string(), point(8.54, 47.37))]);
    let node = node_with(HashMap::from([(
        "venue".to_string(),
        PropertyValue::Object(venue),
    )]));
    let entries = write(&db, &node, &HLC::new(100, 1));
    assert_eq!(live_paths(&entries), vec!["venue.geo".to_string()]);
}

#[test]
fn geometry_inside_an_element_indexes_under_the_dotted_path() {
    let (_dir, db) = test_db();
    let node = node_with(HashMap::from([(
        "hero".to_string(),
        element("map_pin", point(8.54, 47.37)),
    )]));
    let entries = write(&db, &node, &HLC::new(100, 1));
    assert_eq!(live_paths(&entries), vec!["hero.map_pin".to_string()]);
}

#[test]
fn geometry_inside_an_array_of_elements_indexes_per_element() {
    let (_dir, db) = test_db();
    let node = node_with(HashMap::from([(
        "stops".to_string(),
        PropertyValue::Array(vec![
            element("geo", point(8.0, 47.0)),
            element("geo", point(8.1, 47.1)),
        ]),
    )]));
    let entries = write(&db, &node, &HLC::new(100, 1));
    assert_eq!(
        live_paths(&entries),
        vec!["stops.0.geo".to_string(), "stops.1.geo".to_string()]
    );
}

/// A node carrying geometry at three depths gets three independent index
/// namespaces, so a query naming one of them scans only that one.
#[test]
fn one_node_with_three_geometries_gets_three_namespaces() {
    let (_dir, db) = test_db();
    let node = node_with(HashMap::from([
        ("location".to_string(), point(8.0, 47.0)),
        (
            "venue".to_string(),
            PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.1, 47.1))])),
        ),
        (
            "stops".to_string(),
            PropertyValue::Array(vec![element("geo", point(8.2, 47.2))]),
        ),
    ]));
    let entries = write(&db, &node, &HLC::new(100, 1));
    assert_eq!(
        live_paths(&entries),
        vec![
            "location".to_string(),
            "stops.0.geo".to_string(),
            "venue.geo".to_string()
        ]
    );
}

/// THE bug class this pass exists to prevent: a nested geometry that moves must
/// stop matching at its old cell.
#[test]
fn moving_a_nested_geometry_tombstones_the_old_cells() {
    let old = node_with(HashMap::from([(
        "venue".to_string(),
        PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.0, 47.0))])),
    )]));
    let new = node_with(HashMap::from([(
        "venue".to_string(),
        PropertyValue::Object(HashMap::from([("geo".to_string(), point(9.0, 48.0))])),
    )]));
    let entries = update(&old, Some(&new), &HLC::new(200, 1));

    assert!(
        tombstoned_paths(&entries).contains(&"venue.geo".to_string()),
        "the superseded nested geometry must be tombstoned, or the node keeps \
         matching at its old location forever"
    );
    assert!(live_paths(&entries).contains(&"venue.geo".to_string()));
}

/// Removing a nested geometry entirely must tombstone it — otherwise the field
/// keeps matching after the user deleted it.
#[test]
fn removing_a_nested_geometry_tombstones_it() {
    let old = node_with(HashMap::from([(
        "hero".to_string(),
        element("map_pin", point(8.0, 47.0)),
    )]));
    let new = node_with(HashMap::from([(
        "hero".to_string(),
        element("caption", PropertyValue::String("no map".into())),
    )]));
    let entries = update(&old, Some(&new), &HLC::new(200, 1));

    assert_eq!(tombstoned_paths(&entries), vec!["hero.map_pin".to_string()]);
    assert!(live_paths(&entries).is_empty());
}

/// Dropping an array element shortens every later path, so the trailing path
/// must be tombstoned or it stays live pointing at a geometry that is gone.
#[test]
fn shortening_an_array_tombstones_the_trailing_path() {
    let old = node_with(HashMap::from([(
        "stops".to_string(),
        PropertyValue::Array(vec![
            element("geo", point(8.0, 47.0)),
            element("geo", point(8.1, 47.1)),
        ]),
    )]));
    let new = node_with(HashMap::from([(
        "stops".to_string(),
        PropertyValue::Array(vec![element("geo", point(8.0, 47.0))]),
    )]));
    let entries = update(&old, Some(&new), &HLC::new(200, 1));

    assert_eq!(tombstoned_paths(&entries), vec!["stops.1.geo".to_string()]);
    // `stops.0.geo` is unchanged, so it is re-written and NOT tombstoned.
    assert_eq!(live_paths(&entries), vec!["stops.0.geo".to_string()]);
}

/// Deleting the node tombstones every nested path, not just the top-level ones.
#[test]
fn deleting_a_node_tombstones_every_nested_path() {
    let node = node_with(HashMap::from([
        ("location".to_string(), point(8.0, 47.0)),
        ("hero".to_string(), element("map_pin", point(8.1, 47.1))),
        (
            "stops".to_string(),
            PropertyValue::Array(vec![element("geo", point(8.2, 47.2))]),
        ),
    ]));
    let entries = update(&node, None, &HLC::new(200, 1));
    assert_eq!(
        tombstoned_paths(&entries),
        vec![
            "hero.map_pin".to_string(),
            "location".to_string(),
            "stops.0.geo".to_string()
        ]
    );
}

/// An unchanged geometry must NOT be tombstoned: the re-write is byte-identical
/// and a tombstone at the same revision would be pure churn.
#[test]
fn unchanged_nested_geometry_is_not_tombstoned() {
    let node = node_with(HashMap::from([(
        "venue".to_string(),
        PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.0, 47.0))])),
    )]));
    let entries = update(&node, Some(&node), &HLC::new(200, 1));
    assert!(tombstoned_paths(&entries).is_empty());
}

/// The write-cost contract, measured rather than asserted in prose.
///
/// The default profile writes one key per configured precision per geometry path;
/// a tracking profile with two precisions writes two. This is the number the
/// tracking-profile documentation quotes, so it is pinned here.
#[test]
fn per_update_write_cost_scales_with_the_precision_set() {
    fn cost(policy: SpatialPolicy) -> usize {
        let (_dir, db) = test_db();
        let targets = SpatialIndexTargets::from_db(&db).unwrap();
        let ctx = IndexCtx::new("t", "r", "main", "ws");
        let policies = NodeSpatialPolicies::new(
            HashMap::from([("position".to_string(), policy)]),
            SpatialPolicy::default(),
        );
        let node = node_with(HashMap::from([(
            "position".to_string(),
            point(8.54, 47.37),
        )]));
        let mut batch = WriteBatch::default();
        write_node_spatial_indexes(
            &mut batch,
            &targets,
            &ctx,
            &node,
            &HLC::new(100, 1),
            &policies,
        )
        .unwrap();
        commit_and_read(&db, batch).len()
    }

    // Default: the eight-precision INDEX_PRECISIONS_DEFAULT set.
    assert_eq!(cost(SpatialPolicy::default()), 8);

    // Tracking: two precisions chosen for the radii actually queried.
    let tracking = SpatialPolicy {
        precisions: raisin_models::nodes::properties::sorted_precisions(vec![6, 8]),
        ..SpatialPolicy::default()
    };
    assert_eq!(cost(tracking), 2);
}
