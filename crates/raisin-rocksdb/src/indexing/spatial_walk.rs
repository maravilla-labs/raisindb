//! Finding every geometry a node carries, wherever it sits in the property tree.
//!
//! # The gap this closes
//!
//! The spatial writer used to iterate `node.properties` and match a flat
//! `PropertyValue::Geometry`. A geometry nested inside an `Element` (a section
//! field), an `Object` or an `Array` of elements was stored on the node, the
//! index reported itself healthy, and the geometry was **invisible to every
//! spatial query** — the silent-wrong-results shape, in the subsystem that had
//! just spent a pass eliminating it.
//!
//! # What is indexed is decided STRUCTURALLY
//!
//! Every `PropertyValue::Geometry` the walk finds is indexed, wherever it sits.
//! This is a deliberate divergence from the shape-driven fulltext indexer
//! (`jobs/handlers/fulltext/plan.rs`), and the reason is the hot path: the
//! spatial writer is **synchronous**, takes `&mut WriteBatch`, and must stay
//! callable from the replication apply path, which cannot await and must not
//! read. Resolving an `ElementType` here would need exactly the read that
//! `jobs/handlers/fulltext/batch.rs` records as reliably deadlocking against an
//! open transaction.
//!
//! The failure mode of structural is EXTRA index entries. The failure mode of
//! shape-driven-on-the-hot-path is a DEADLOCK, and of shape-driven-resolved-wrong
//! is SILENT INVISIBILITY — the bug being fixed. **Shape drives policy, not
//! selection**, and policy is resolved off the hot path by
//! [`crate::spatial_state::SpatialStateStore`].
//!
//! # Path format
//!
//! See [`super::property_walk`]. A top-level path is byte-identical to the bare
//! property name, so existing entries stay valid with no migration.

use super::property_walk::walk_properties;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use std::collections::HashMap;

/// Hard ceiling on geometry paths indexed for one node.
///
/// "Index every geometry, wherever it sits" is unbounded by construction: an
/// array of 10,000 elements each holding a point would emit 80,000 index keys in
/// one batch under the default eight-precision policy. The cap turns that into a
/// bounded write plus a loud warning naming exactly what was dropped.
///
/// 64 is chosen to be far above any hand-modelled document (a page with a dozen
/// map sections is 12) and far below anything that could stall a batch.
pub const MAX_GEOMETRY_PATHS_PER_NODE: usize = 64;

/// Every geometry on a node, with its dot-format property path.
///
/// Sorted by path and therefore deterministic — which the cap depends on, since
/// an unstable order would silently index a different subset on every write.
///
/// This is the canonical walker for the spatial index: the writer, the
/// tombstoner, the policy resolver and the rebuild job all use it, so the path a
/// key embeds and the path a tombstone targets cannot disagree.
pub fn walk_geometries(properties: &HashMap<String, PropertyValue>) -> Vec<(String, &GeoJson)> {
    // Geometry-ness is a property of the VALUE, so the cursor is not consulted —
    // selection here is structural by design (see the module header).
    walk_properties(properties, |_cursor, value| match value {
        PropertyValue::Geometry(geometry) => Some(geometry),
        _ => None,
    })
}

/// The geometries of one node after the [`MAX_GEOMETRY_PATHS_PER_NODE`] cap.
pub struct WalkedGeometries<'a> {
    /// The paths that will be indexed, sorted.
    pub indexed: Vec<(String, &'a GeoJson)>,
    /// Paths dropped by the cap. Empty in every normal case.
    pub dropped: Vec<String>,
}

impl WalkedGeometries<'_> {
    /// Whether the cap discarded anything, i.e. whether this node's spatial index
    /// is knowingly incomplete.
    pub fn overflowed(&self) -> bool {
        !self.dropped.is_empty()
    }
}

/// [`walk_geometries`] with the per-node cap applied, warning on overflow.
///
/// The warning names the node and every dropped path, because an operator whose
/// query silently misses a geometry needs to be able to find out why from the
/// log alone.
pub fn walk_geometries_capped<'a>(
    properties: &'a HashMap<String, PropertyValue>,
    node_id: &str,
) -> WalkedGeometries<'a> {
    let mut all = walk_geometries(properties);
    if all.len() <= MAX_GEOMETRY_PATHS_PER_NODE {
        return WalkedGeometries {
            indexed: all,
            dropped: Vec::new(),
        };
    }

    let dropped: Vec<String> = all
        .split_off(MAX_GEOMETRY_PATHS_PER_NODE)
        .into_iter()
        .map(|(path, _)| path)
        .collect();

    tracing::warn!(
        node_id = %node_id,
        indexed = all.len(),
        dropped = dropped.len(),
        dropped_paths = %dropped.join(", "),
        "Node carries more than {} geometry properties; the remainder are NOT \
         spatially indexed and will not match ST_DWITHIN / ST_DISTANCE on their \
         paths. Model the extra geometries on child nodes instead.",
        MAX_GEOMETRY_PATHS_PER_NODE
    );

    WalkedGeometries {
        indexed: all,
        dropped,
    }
}

/// The property PATHS [`walk_geometries_capped`] would index, without the
/// geometries and without re-emitting the overflow warning.
///
/// # Why this exists
///
/// Writing the index entries is only half of indexing a geometry property. The
/// other half is the per-property **index-state record**, which is what
/// `PhysicalPlanner::spatial_availability` reads to decide whether the index may
/// answer a query at all. Every write path therefore does two things: stage the
/// entries via [`super::write_node_spatial_indexes`], then `ensure_for_write` a
/// state record per property.
///
/// Those two loops MUST range over the same set. They did not: the writer walked
/// the whole property tree while the state-record loop iterated `node.properties`
/// flat, so a nested geometry got real index entries and NO state record — and a
/// missing record means `NotBuilt`, which means the planner routes every nested
/// query onto `build_spatial_fallback_scan` forever. Correct rows, a permanent
/// full scan, entries that are never read, and nothing anywhere reporting a
/// problem. Sharing the walker (and the cap, and its order) is what keeps the two
/// halves in step.
pub fn indexed_geometry_paths(properties: &HashMap<String, PropertyValue>) -> Vec<String> {
    let mut paths: Vec<String> = walk_geometries(properties)
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    // Same cap and same (sorted) order as `walk_geometries_capped`, so a node over
    // the ceiling gets state records for exactly the paths that got entries.
    paths.truncate(MAX_GEOMETRY_PATHS_PER_NODE);
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Element;

    fn point(lon: f64, lat: f64) -> PropertyValue {
        PropertyValue::Geometry(GeoJson::point(lon, lat))
    }

    fn paths(properties: &HashMap<String, PropertyValue>) -> Vec<String> {
        walk_geometries(properties)
            .into_iter()
            .map(|(p, _)| p)
            .collect()
    }

    #[test]
    fn top_level_geometry_path_is_unchanged_from_the_flat_format() {
        let mut props = HashMap::new();
        props.insert("location".into(), point(8.54, 47.37));
        // Byte-identical to what the old flat loop emitted: this is what makes
        // existing index entries valid without migration.
        assert_eq!(paths(&props), vec!["location".to_string()]);
    }

    #[test]
    fn geometry_two_levels_deep_in_an_object() {
        let mut venue = HashMap::new();
        venue.insert("geo".into(), point(8.54, 47.37));
        let mut props = HashMap::new();
        props.insert("venue".into(), PropertyValue::Object(venue));
        assert_eq!(paths(&props), vec!["venue.geo".to_string()]);
    }

    #[test]
    fn geometry_three_levels_deep() {
        let mut level3 = HashMap::new();
        level3.insert("geo".into(), point(8.54, 47.37));
        let mut level2 = HashMap::new();
        level2.insert("main".into(), PropertyValue::Object(level3));
        let mut props = HashMap::new();
        props.insert("venue".into(), PropertyValue::Object(level2));
        assert_eq!(paths(&props), vec!["venue.main.geo".to_string()]);
    }

    #[test]
    fn geometry_inside_an_element_content_field() {
        let mut content = HashMap::new();
        content.insert("map_pin".into(), point(8.54, 47.37));
        let mut props = HashMap::new();
        props.insert(
            "hero".into(),
            PropertyValue::Element(Element {
                uuid: "el-1".into(),
                element_type: "demo:MapSection".into(),
                content,
            }),
        );
        assert_eq!(paths(&props), vec!["hero.map_pin".to_string()]);
    }

    #[test]
    fn geometry_inside_an_array_of_elements_uses_zero_based_indices() {
        let stop = |lon: f64| {
            let mut content = HashMap::new();
            content.insert("geo".into(), point(lon, 47.0));
            PropertyValue::Element(Element {
                uuid: String::new(),
                element_type: "demo:Stop".into(),
                content,
            })
        };
        let mut props = HashMap::new();
        props.insert(
            "stops".into(),
            PropertyValue::Array(vec![stop(8.0), stop(8.1), stop(8.2)]),
        );
        assert_eq!(
            paths(&props),
            vec![
                "stops.0.geo".to_string(),
                "stops.1.geo".to_string(),
                "stops.2.geo".to_string(),
            ]
        );
    }

    /// The owner's motivating shape: one node, three geometries, three depths.
    #[test]
    fn one_node_with_three_geometries_at_different_depths() {
        let mut venue = HashMap::new();
        venue.insert("geo".into(), point(8.1, 47.1));

        let mut content = HashMap::new();
        content.insert("geo".into(), point(8.2, 47.2));

        let mut props = HashMap::new();
        props.insert("location".into(), point(8.0, 47.0));
        props.insert("venue".into(), PropertyValue::Object(venue));
        props.insert(
            "stops".into(),
            PropertyValue::Array(vec![PropertyValue::Element(Element {
                uuid: String::new(),
                element_type: "demo:Stop".into(),
                content,
            })]),
        );

        assert_eq!(
            paths(&props),
            vec![
                "location".to_string(),
                "stops.0.geo".to_string(),
                "venue.geo".to_string(),
            ]
        );
    }

    #[test]
    fn non_geometry_leaves_are_ignored() {
        let mut venue = HashMap::new();
        venue.insert("name".into(), PropertyValue::String("Hall".into()));
        venue.insert("geo".into(), point(8.54, 47.37));
        let mut props = HashMap::new();
        props.insert("venue".into(), PropertyValue::Object(venue));
        props.insert("title".into(), PropertyValue::String("x".into()));
        assert_eq!(paths(&props), vec!["venue.geo".to_string()]);
    }

    #[test]
    fn cap_keeps_the_first_n_paths_and_reports_the_rest() {
        let mut items = Vec::new();
        for i in 0..(MAX_GEOMETRY_PATHS_PER_NODE + 5) {
            let mut content = HashMap::new();
            content.insert("geo".into(), point(8.0 + i as f64 * 0.001, 47.0));
            items.push(PropertyValue::Element(Element {
                uuid: String::new(),
                element_type: "demo:Stop".into(),
                content,
            }));
        }
        let mut props = HashMap::new();
        props.insert("stops".into(), PropertyValue::Array(items));

        let walked = walk_geometries_capped(&props, "node-1");
        assert!(walked.overflowed());
        assert_eq!(walked.indexed.len(), MAX_GEOMETRY_PATHS_PER_NODE);
        assert_eq!(walked.dropped.len(), 5);
        // Deterministic: the same subset every time.
        let again = walk_geometries_capped(&props, "node-1");
        assert_eq!(
            walked.indexed.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            again.indexed.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cap_is_inert_below_the_ceiling() {
        let mut props = HashMap::new();
        props.insert("location".into(), point(8.54, 47.37));
        let walked = walk_geometries_capped(&props, "node-1");
        assert!(!walked.overflowed());
        assert_eq!(walked.indexed.len(), 1);
    }
}
