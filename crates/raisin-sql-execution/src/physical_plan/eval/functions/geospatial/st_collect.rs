//! ST_COLLECT - gather two geometries without merging them.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Geometry, MultiLineString, MultiPoint, MultiPolygon};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_pair};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_COLLECT(geometry1, geometry2) -> GEOMETRY";

/// Both geometries in one value, kept separate.
///
/// # SQL Signature
/// `ST_COLLECT(geometry1, geometry2) -> GEOMETRY`
///
/// # ST_COLLECT versus ST_UNION
/// `ST_COLLECT` is a **container**: it does no geometric work, so overlapping
/// inputs stay overlapping and the total area is the sum with the overlap counted
/// twice. [`ST_UNION`](super::StUnionFunction) dissolves the boundaries and counts
/// the overlap once. Collecting is far cheaper and is what you want before an
/// `ST_ENVELOPE` or an `ST_CONVEXHULL`.
///
/// # Behaviour
/// * Two geometries of the same type collapse to the matching homogeneous
///   `Multi*` — two Points give a `MultiPoint`, not a GeometryCollection — which is
///   PostGIS's behaviour and keeps the result usable by type-specific functions.
///   The previous implementation always produced a GeometryCollection.
/// * Existing `Multi*` inputs are flattened rather than nested, so collecting
///   repeatedly grows one collection instead of building a tower of them.
/// * Mixed types give a `GeometryCollection`.
/// * Collecting with the empty geometry returns the other argument.
/// * `NULL` in, `NULL` out.
pub struct StCollectFunction;

impl SqlFunction for StCollectFunction {
    fn name(&self) -> &str {
        "ST_COLLECT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_COLLECT", SIGNATURE, args, 2)?;
        let Some((a, b)) = geom_pair("ST_COLLECT", args, row)? else {
            return Ok(Literal::Null);
        };

        let mut members: Vec<Geometry<f64>> = Vec::new();
        flatten(&a.geometry, &mut members);
        flatten(&b.geometry, &mut members);
        derived_result(collect(members), &a)
    }
}

/// Expand `Multi*` and `GeometryCollection` into their members so collections do
/// not nest.
fn flatten(g: &Geometry<f64>, out: &mut Vec<Geometry<f64>>) {
    match g {
        Geometry::MultiPoint(mp) => out.extend(mp.0.iter().map(|p| Geometry::Point(*p))),
        Geometry::MultiLineString(mls) => {
            out.extend(mls.0.iter().cloned().map(Geometry::LineString))
        }
        Geometry::MultiPolygon(mp) => out.extend(mp.0.iter().cloned().map(Geometry::Polygon)),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|m| flatten(m, out)),
        other => out.push(other.clone()),
    }
}

/// Choose the narrowest container for a set of single geometries.
fn collect(members: Vec<Geometry<f64>>) -> Geometry<f64> {
    if members.is_empty() {
        return Geometry::GeometryCollection(Default::default());
    }

    let points: Vec<_> = members
        .iter()
        .filter_map(|m| match m {
            Geometry::Point(p) => Some(*p),
            _ => None,
        })
        .collect();
    if points.len() == members.len() {
        return Geometry::MultiPoint(MultiPoint(points));
    }

    let lines: Vec<_> = members
        .iter()
        .filter_map(|m| match m {
            Geometry::LineString(ls) => Some(ls.clone()),
            _ => None,
        })
        .collect();
    if lines.len() == members.len() {
        return Geometry::MultiLineString(MultiLineString(lines));
    }

    let polygons: Vec<_> = members
        .iter()
        .filter_map(|m| match m {
            Geometry::Polygon(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if polygons.len() == members.len() {
        return Geometry::MultiPolygon(MultiPolygon(polygons));
    }

    Geometry::GeometryCollection(members.into())
}
