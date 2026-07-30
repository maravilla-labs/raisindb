//! Resolving a **nested** geometry property path against a row.
//!
//! # The failure this prevents
//!
//! The spatial index now walks nested properties and indexes a geometry wherever
//! it sits (`raisin_rocksdb::indexing::spatial_walk`). The query side already
//! parsed the addressing for free: `extract_geometry_source` takes the key of
//! `properties->>'venue.geo'` verbatim, so the planner produces a scan over the
//! index namespace `venue.geo` with no grammar change.
//!
//! The ROW-LEVEL side did not. `properties->>'venue.geo'` is an ordinary JSON key
//! lookup, and a JSON object has no key literally called `venue.geo`, so it
//! evaluated to NULL and `ST_DWITHIN(NULL, …)` returned NULL — **zero rows**.
//!
//! That matters far more than it sounds, because the row-level path is the
//! FALLBACK: it is exactly what runs before a rebuild has drained, when the index
//! for a nested path is not yet `Ready`. Without this module the migration window
//! would report "slow but correct" in the log while returning nothing — turning
//! the bug being fixed into a subtler one.
//!
//! # The format is the walker's, exactly
//!
//! Segments separated by `.`; array elements addressed by zero-based index
//! (`stops.0.geo`). Identical to the format the index key embeds, because the
//! query and the writer must name the same thing. `[]` is additionally accepted
//! **in a query only** as a wildcard over every element of an array.
//!
//! A direct JSON key wins over a path walk: if a node genuinely has a top-level
//! property named `venue.geo`, `properties->>'venue.geo'` reads it, exactly as it
//! did before. Nested resolution is a fallback, never an override. (A property
//! name containing a dot is therefore ambiguous and is not disambiguated — the
//! same limitation the reference index has always carried.)

use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};
use serde_json::Value;

use crate::physical_plan::executor::Row;

/// One geometry found at a concrete path.
#[derive(Debug, Clone)]
pub(super) struct MatchedGeometry {
    /// The CONCRETE dotted path, with array indices resolved (`stops.3.geo`).
    /// This is what a wildcard query reports as the field that matched.
    pub path: String,
    pub geometry: Value,
}

/// The geometry a spatial predicate actually matched on, and how far away it is.
///
/// `path` is the CONCRETE dotted path (`stops.3.geo`), never the wildcard the
/// query was written with — answering "which one matched" is the entire reason a
/// wildcard path exists.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NearestGeometry {
    pub path: String,
    pub distance_meters: f64,
}

/// The closest geometry to `(center_lon, center_lat)` among those `pattern`
/// addresses inside `properties`.
///
/// This is the row-level source of the `__distance` / `__matched_path`
/// pseudo-columns on the spatial FALLBACK path, where no index entry exists to
/// read a distance off. It deliberately mirrors `ST_DWITHIN`'s own definition
/// for a wildcard — the MINIMUM over the matched geometries — so the annotated
/// distance is the distance the predicate was decided on, not a second opinion.
///
/// A tie resolves to the lexicographically smallest concrete path, so a node with
/// two equidistant stops annotates deterministically.
///
/// Returns `None` when the pattern addresses no geometry on this row.
pub(crate) fn nearest_geometry(
    properties: &PropertyValue,
    pattern: &str,
    center_lon: f64,
    center_lat: f64,
) -> Option<NearestGeometry> {
    let center = super::helpers::point_to_geojson(center_lon, center_lat);
    let mut best: Option<NearestGeometry> = None;

    for matched in matched_geometries(properties, pattern) {
        let Ok(distance) = super::helpers::compute_haversine_distance(&matched.geometry, &center)
        else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(current) => {
                distance < current.distance_meters
                    || (distance == current.distance_meters && matched.path < current.path)
            }
        };
        if better {
            best = Some(NearestGeometry {
                path: matched.path,
                distance_meters: distance,
            });
        }
    }

    best
}

/// Every geometry `pattern` addresses in `properties`.
///
/// A FLAT key (`location`) is an ordinary top-level property lookup — the nested
/// walker would reject it, and the flat case is the common one. Anything with a
/// `.` or a `[]` goes through the same walker the index writer uses.
fn matched_geometries(properties: &PropertyValue, pattern: &str) -> Vec<MatchedGeometry> {
    if !is_nested_path(pattern) {
        let value = match properties {
            PropertyValue::Object(map) => map.get(pattern),
            PropertyValue::Element(element) => element.content.get(pattern),
            _ => None,
        };
        return match value {
            Some(PropertyValue::Geometry(geometry)) => serde_json::to_value(geometry)
                .ok()
                .map(|json| {
                    vec![MatchedGeometry {
                        path: pattern.to_string(),
                        geometry: json,
                    }]
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
    }

    let mut out = Vec::new();
    collect(properties, &segments(pattern), 0, String::new(), &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Whether a property key is a nested path rather than a plain property name.
///
/// A key with no `.` and no `[]` cannot address anything the ordinary JSON
/// lookup does not already reach, so the fallback is skipped entirely for the
/// overwhelmingly common flat case.
pub(super) fn is_nested_path(key: &str) -> bool {
    key.contains('.') || key.contains("[]")
}

// NOTE: "is this a wildcard path?" deliberately has ONE implementation, in
// `raisin_models::nodes::properties::is_wildcard_property_path`, which the
// planner uses to refuse the index path. A second copy here would be a second
// place for the two halves to disagree about what a wildcard is.

/// The `(column, path)` a geometry argument addresses, when it is written as a
/// JSON property access — `properties->>'venue.geo'` or `properties->'venue.geo'`,
/// with or without a `CAST(... AS GEOMETRY)` wrapper.
///
/// Deliberately narrower than the planner's `extract_geometry_source`: a BARE
/// column is NOT accepted here. `Expr::Column { table, column }` maps a bare
/// dotted identifier to a *qualified column reference* (`table = venue`,
/// `column = geo`), not to a property path, so `ST_DWITHIN(venue.geo, …)` means
/// something else entirely. Nested geometry MUST be written as
/// `properties->>'venue.geo'`.
pub(super) fn property_path_source(expr: &Expr) -> Option<(String, String, String)> {
    match expr {
        Expr::Cast {
            expr: inner,
            target_type: raisin_sql::analyzer::DataType::Geometry,
        } => property_path_source(&inner.expr),
        Expr::JsonExtractText { object, key } | Expr::JsonExtract { object, key } => {
            let Expr::Column { table, column } = &object.expr else {
                return None;
            };
            let Expr::Literal(Literal::Text(path)) = &key.expr else {
                return None;
            };
            Some((table.clone(), column.clone(), path.clone()))
        }
        _ => None,
    }
}

/// Every geometry in `row`'s property tree matching `pattern`.
///
/// Returns an empty vector when the argument is not a property access, when the
/// row carries no such column, or when nothing matches. Results are sorted by
/// concrete path, so a tie between two equally-distant array elements resolves to
/// the lexicographically smallest one and the answer is deterministic.
pub(super) fn resolve_from_row(arg: &TypedExpr, row: &Row) -> Vec<MatchedGeometry> {
    let Some((table, column, pattern)) = property_path_source(&arg.expr) else {
        return Vec::new();
    };
    if !is_nested_path(&pattern) {
        return Vec::new();
    }

    let Some(properties) = lookup_column(row, &table, &column) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    collect(properties, &segments(&pattern), 0, String::new(), &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Find the `properties` column however the row happens to be qualified.
///
/// A scan emits fully qualified names (`nodes.properties`); a post-projection row
/// may carry the bare name. Mirrors `eval_column`'s own lookup ladder.
fn lookup_column<'a>(row: &'a Row, table: &str, column: &str) -> Option<&'a PropertyValue> {
    if !table.is_empty() {
        if let Some(value) = row.get(&format!("{}.{}", table, column)) {
            return Some(value);
        }
    }
    row.get(column).or_else(|| row.get_by_unqualified(column))
}

/// One step of the pattern.
enum Segment<'a> {
    /// A literal key, or a literal array index written as digits.
    Key(&'a str),
    /// `[]` — every element of the array at this position.
    AnyIndex,
}

/// Split a pattern into segments, lifting a trailing `[]` off a key.
///
/// `stops[].geo` is `[Key("stops"), AnyIndex, Key("geo")]`, so the wildcard
/// spelling and the concrete spelling (`stops.0.geo`) walk the same tree the same
/// way.
fn segments(pattern: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    for raw in pattern.split('.') {
        let mut name = raw;
        let mut wildcards = 0;
        while let Some(stripped) = name.strip_suffix("[]") {
            name = stripped;
            wildcards += 1;
        }
        if !name.is_empty() {
            out.push(Segment::Key(name));
        }
        for _ in 0..wildcards {
            out.push(Segment::AnyIndex);
        }
    }
    out
}

/// Walk `value` against `pattern[depth..]`, appending every geometry reached.
///
/// Descends `Object`, `Element` (whose fields live in `content`) and `Array` — the
/// same three containers `walk_geometries` descends, so what the index holds and
/// what a row scan finds are the same set.
fn collect(
    value: &PropertyValue,
    pattern: &[Segment<'_>],
    depth: usize,
    path: String,
    out: &mut Vec<MatchedGeometry>,
) {
    if depth == pattern.len() {
        if let PropertyValue::Geometry(geometry) = value {
            if let Ok(json) = serde_json::to_value(geometry) {
                out.push(MatchedGeometry {
                    path,
                    geometry: json,
                });
            }
        }
        return;
    }

    let join = |path: &str, segment: &str| -> String {
        if path.is_empty() {
            segment.to_string()
        } else {
            format!("{}.{}", path, segment)
        }
    };

    match (&pattern[depth], value) {
        (Segment::Key(key), PropertyValue::Object(obj)) => {
            if let Some(child) = obj.get(*key) {
                collect(child, pattern, depth + 1, join(&path, key), out);
            }
        }
        (Segment::Key(key), PropertyValue::Element(element)) => {
            if let Some(child) = element.content.get(*key) {
                collect(child, pattern, depth + 1, join(&path, key), out);
            }
        }
        // A concrete array index written in the path (`stops.0.geo`).
        (Segment::Key(key), PropertyValue::Array(items)) => {
            if let Ok(index) = key.parse::<usize>() {
                if let Some(child) = items.get(index) {
                    collect(child, pattern, depth + 1, join(&path, key), out);
                }
            }
        }
        (Segment::AnyIndex, PropertyValue::Array(items)) => {
            for (index, child) in items.iter().enumerate() {
                collect(
                    child,
                    pattern,
                    depth + 1,
                    join(&path, &index.to_string()),
                    out,
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Element;
    use raisin_models::nodes::properties::GeoJson;
    use std::collections::HashMap;

    fn point(lon: f64, lat: f64) -> PropertyValue {
        PropertyValue::Geometry(GeoJson::point(lon, lat))
    }

    fn element(field: &str, value: PropertyValue) -> PropertyValue {
        PropertyValue::Element(Element {
            uuid: String::new(),
            element_type: "demo:Section".into(),
            content: HashMap::from([(field.to_string(), value)]),
        })
    }

    fn row_with(properties: HashMap<String, PropertyValue>) -> Row {
        let mut row = Row::new();
        row.insert("nodes.properties".into(), PropertyValue::Object(properties));
        row
    }

    fn resolve(row: &Row, pattern: &str) -> Vec<String> {
        let arg = TypedExpr {
            expr: Expr::JsonExtractText {
                object: Box::new(TypedExpr {
                    expr: Expr::Column {
                        table: "nodes".into(),
                        column: "properties".into(),
                    },
                    data_type: raisin_sql::analyzer::DataType::JsonB,
                }),
                key: Box::new(TypedExpr {
                    expr: Expr::Literal(Literal::Text(pattern.to_string())),
                    data_type: raisin_sql::analyzer::DataType::Text,
                }),
            },
            data_type: raisin_sql::analyzer::DataType::Text,
        };
        resolve_from_row(&arg, row)
            .into_iter()
            .map(|m| m.path)
            .collect()
    }

    #[test]
    fn object_nested_path_resolves() {
        let row = row_with(HashMap::from([(
            "venue".to_string(),
            PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.0, 47.0))])),
        )]));
        assert_eq!(resolve(&row, "venue.geo"), vec!["venue.geo".to_string()]);
    }

    #[test]
    fn element_nested_path_resolves() {
        let row = row_with(HashMap::from([(
            "hero".to_string(),
            element("map_pin", point(8.0, 47.0)),
        )]));
        assert_eq!(
            resolve(&row, "hero.map_pin"),
            vec!["hero.map_pin".to_string()]
        );
    }

    #[test]
    fn concrete_array_index_resolves_exactly_one() {
        let row = row_with(HashMap::from([(
            "stops".to_string(),
            PropertyValue::Array(vec![
                element("geo", point(8.0, 47.0)),
                element("geo", point(8.1, 47.1)),
            ]),
        )]));
        assert_eq!(
            resolve(&row, "stops.1.geo"),
            vec!["stops.1.geo".to_string()]
        );
    }

    #[test]
    fn wildcard_resolves_every_element_in_path_order() {
        let row = row_with(HashMap::from([(
            "stops".to_string(),
            PropertyValue::Array(vec![
                element("geo", point(8.0, 47.0)),
                element("geo", point(8.1, 47.1)),
                element("geo", point(8.2, 47.2)),
            ]),
        )]));
        assert_eq!(
            resolve(&row, "stops[].geo"),
            vec![
                "stops.0.geo".to_string(),
                "stops.1.geo".to_string(),
                "stops.2.geo".to_string()
            ]
        );
    }

    #[test]
    fn three_levels_deep_resolves() {
        let row = row_with(HashMap::from([(
            "venue".to_string(),
            PropertyValue::Object(HashMap::from([(
                "main".to_string(),
                PropertyValue::Object(HashMap::from([("geo".to_string(), point(8.0, 47.0))])),
            )])),
        )]));
        assert_eq!(
            resolve(&row, "venue.main.geo"),
            vec!["venue.main.geo".to_string()]
        );
    }

    #[test]
    fn a_path_that_matches_nothing_resolves_to_nothing() {
        let row = row_with(HashMap::from([("location".to_string(), point(8.0, 47.0))]));
        assert!(resolve(&row, "venue.geo").is_empty());
    }

    #[test]
    fn a_non_geometry_leaf_is_not_matched() {
        let row = row_with(HashMap::from([(
            "venue".to_string(),
            PropertyValue::Object(HashMap::from([(
                "geo".to_string(),
                PropertyValue::String("not a geometry".into()),
            )])),
        )]));
        assert!(resolve(&row, "venue.geo").is_empty());
    }

    #[test]
    fn a_flat_key_is_left_to_the_ordinary_json_lookup() {
        let row = row_with(HashMap::from([("location".to_string(), point(8.0, 47.0))]));
        assert!(resolve(&row, "location").is_empty());
    }
}
