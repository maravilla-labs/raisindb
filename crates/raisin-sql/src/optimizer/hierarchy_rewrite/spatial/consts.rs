//! Constant-folding and envelope arithmetic for spatial query geometries.
//!
//! Everything here operates on *constant* operands only — the side of a spatial
//! predicate the planner is allowed to evaluate at plan time. `raisin-sql`
//! deliberately carries no geometry dependency, so the GeoJSON walking is done
//! generically over `serde_json::Value` (find any array whose first two entries
//! are numbers and treat it as a position). That handles `MultiPolygon`,
//! `GeometryCollection`, `Feature` and `FeatureCollection` without a
//! seven-arm match, which is the same "stop matching per geometry type" lesson
//! the ST_* evaluators learned.

use crate::analyzer::{Expr, Literal, TypedExpr};
use serde_json::Value;

/// Mean Earth radius in metres (GRS80 mean, matching the spatial index's own
/// Haversine post-filter and `raisin_geometry::EARTH_MEAN_RADIUS_METERS`).
///
/// Duplicated here rather than imported because `raisin-sql` has no geometry
/// dependency; it is used only to inflate a radius conservatively, so a small
/// disagreement would cost candidates, never correctness.
const EARTH_MEAN_RADIUS_METERS: f64 = 6_371_008.8;

/// Fold an expression to a constant GeoJSON geometry, or `None` if it is not
/// constant-foldable.
pub fn const_geometry(expr: &TypedExpr) -> Option<Value> {
    match &expr.expr {
        Expr::Literal(Literal::Geometry(v)) => Some(v.clone()),
        Expr::Literal(Literal::JsonB(v)) if looks_like_geojson(v) => Some(v.clone()),
        Expr::Literal(Literal::Text(s)) => serde_json::from_str::<Value>(s)
            .ok()
            .filter(looks_like_geojson),
        // `CAST('<geojson>' AS GEOMETRY)` and friends: the cast is a no-op over a
        // constant, so look through it.
        Expr::Cast { expr: inner, .. } => const_geometry(inner),
        Expr::Function { name, args, .. } => {
            let upper = name.to_uppercase();
            match upper.as_str() {
                "ST_POINT" | "ST_MAKEPOINT" if args.len() == 2 || args.len() == 3 => {
                    let lon = numeric_literal(&args[0])?;
                    let lat = numeric_literal(&args[1])?;
                    if !lon.is_finite() || !lat.is_finite() {
                        return None;
                    }
                    Some(serde_json::json!({ "type": "Point", "coordinates": [lon, lat] }))
                }
                "ST_GEOMFROMGEOJSON" | "ST_GEOMFROMTEXT" if args.len() == 1 => {
                    const_geometry(&args[0])
                }
                // Metadata-only on the coordinates — but the LABEL matters, and
                // dropping it here was a silent-wrong-answer bug: the spatial
                // index is keyed on WGS84 lon/lat, so
                // `ST_SETSRID(ST_POINT(2683000, 1247000), 2056)` folded to a bare
                // point and was planned as if 2683000 were a longitude. Keep the
                // label; `spatial::query_center_is_wgs84` is what decides whether
                // the index path is eligible.
                "ST_SETSRID" if args.len() == 2 => {
                    let mut inner = const_geometry(&args[0])?;
                    let srid = numeric_literal(&args[1])?;
                    if !(srid.is_finite() && srid > 0.0 && srid <= u32::MAX as f64) {
                        return None;
                    }
                    inner.as_object_mut()?.insert(
                        "srid".to_string(),
                        Value::from(srid.round().max(0.0) as u64),
                    );
                    Some(inner)
                }
                "ST_MAKEENVELOPE" if args.len() == 4 || args.len() == 5 => {
                    let min_x = numeric_literal(&args[0])?;
                    let min_y = numeric_literal(&args[1])?;
                    let max_x = numeric_literal(&args[2])?;
                    let max_y = numeric_literal(&args[3])?;
                    Some(serde_json::json!({
                        "type": "Polygon",
                        "coordinates": [[
                            [min_x, min_y], [max_x, min_y],
                            [max_x, max_y], [min_x, max_y],
                            [min_x, min_y],
                        ]],
                    }))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The EPSG code a constant query geometry is expressed in, or `None` when it
/// carries no `srid` member (which means WGS84 by convention everywhere in
/// RaisinDB).
///
/// Accepts the same spellings as `raisin_geometry::srid::srid_member` — a bare
/// number, `"4326"`, `"EPSG:4326"`, `"SRID=4326"` — without taking the geometry
/// dependency this crate deliberately avoids. An **unparseable** member returns
/// `Some(0)`, which no CRS uses, so the caller treats it as "not WGS84" and
/// declines the index rather than guessing.
pub fn const_geometry_srid(v: &Value) -> Option<u32> {
    match v.get("srid") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => Some(n.as_u64().unwrap_or(0).min(u32::MAX as u64) as u32),
        Some(Value::String(s)) => {
            let t = s.trim();
            let digits = t
                .rsplit_once(':')
                .or_else(|| t.rsplit_once('='))
                .map(|(_, tail)| tail)
                .unwrap_or(t)
                .trim();
            Some(digits.parse::<u32>().unwrap_or(0))
        }
        Some(_) => Some(0),
    }
}

/// Cheap structural test: a GeoJSON object carries a `type` plus either
/// `coordinates`, `geometries`, `geometry` or `features`.
fn looks_like_geojson(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.contains_key("type")
        && (obj.contains_key("coordinates")
            || obj.contains_key("geometries")
            || obj.contains_key("geometry")
            || obj.contains_key("features"))
}

/// A numeric literal as `f64`. Accepts the integer literal spellings too, so
/// `ST_DWITHIN(loc, ST_POINT(8, 47), 500)` behaves like the `8.0 / 47.0 / 500.0`
/// spelling instead of silently declining the index.
pub fn numeric_literal(expr: &TypedExpr) -> Option<f64> {
    match &expr.expr {
        Expr::Literal(Literal::Double(f)) => Some(*f),
        Expr::Literal(Literal::Int(i)) => Some(*i as f64),
        Expr::Literal(Literal::BigInt(i)) => Some(*i as f64),
        // A bound parameter that `params::substitute` already replaced arrives as
        // a plain literal, so nothing extra is needed for `$1`. An UNsubstituted
        // Parameter is deliberately rejected: planning against a placeholder
        // would bake the wrong window into the plan.
        _ => None,
    }
}

/// `[min_x, min_y, max_x, max_y]` over every position in a GeoJSON value.
///
/// Walks `coordinates` / `geometries` / `geometry` / `features` generically
/// rather than matching on the seven geometry types, so `MultiPolygon`,
/// `GeometryCollection`, `Feature` and `FeatureCollection` all work.
pub fn geojson_envelope(v: &Value) -> Option<[f64; 4]> {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    accumulate(v, &mut bounds);
    if bounds.iter().all(|b| b.is_finite()) {
        Some(bounds)
    } else {
        None
    }
}

fn accumulate(v: &Value, bounds: &mut [f64; 4]) {
    let Some(obj) = v.as_object() else {
        return;
    };
    if let Some(coords) = obj.get("coordinates") {
        walk_positions(coords, bounds);
    }
    for key in ["geometries", "features"] {
        if let Some(Value::Array(items)) = obj.get(key) {
            for item in items {
                accumulate(item, bounds);
            }
        }
    }
    if let Some(geometry) = obj.get("geometry") {
        accumulate(geometry, bounds);
    }
}

fn walk_positions(v: &Value, bounds: &mut [f64; 4]) {
    let Value::Array(items) = v else {
        return;
    };
    // A position is an array whose first two entries are numbers. Anything else
    // is a nesting level (ring, part, ...).
    if items.len() >= 2 && items[0].is_number() && items[1].is_number() {
        if let (Some(x), Some(y)) = (items[0].as_f64(), items[1].as_f64()) {
            if x.is_finite() && y.is_finite() {
                bounds[0] = bounds[0].min(x);
                bounds[1] = bounds[1].min(y);
                bounds[2] = bounds[2].max(x);
                bounds[3] = bounds[3].max(y);
            }
        }
        return;
    }
    for item in items {
        walk_positions(item, bounds);
    }
}

/// Centre of an envelope, in the same `(lon, lat)` order as everything else.
pub fn envelope_center(envelope: &[f64; 4]) -> (f64, f64) {
    (
        (envelope[0] + envelope[2]) / 2.0,
        (envelope[1] + envelope[3]) / 2.0,
    )
}

/// Distance in metres from an envelope's centre to its farthest corner.
///
/// Used to inflate a radius when a non-point query geometry is reduced to a
/// single centre. Deliberately an over-estimate (it uses the widest parallel of
/// the box for the longitude leg), because under-estimating would turn the index
/// scan from a superset into a subset.
pub fn envelope_circumradius_meters(envelope: &[f64; 4]) -> f64 {
    let [min_x, min_y, max_x, max_y] = *envelope;
    if min_x == max_x && min_y == max_y {
        return 0.0;
    }
    let d_lat = (max_y - min_y).to_radians() / 2.0;
    // Widest parallel: the latitude of the box closest to the equator.
    let widest_lat = if min_y <= 0.0 && max_y >= 0.0 {
        0.0
    } else {
        min_y.abs().min(max_y.abs())
    };
    let d_lon = (max_x - min_x).to_radians() / 2.0 * widest_lat.to_radians().cos();
    EARTH_MEAN_RADIUS_METERS * (d_lat * d_lat + d_lon * d_lon).sqrt()
}
