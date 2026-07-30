//! ST_DISTANCE - minimum distance between two geometries.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_pair;
use super::measure;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_DISTANCE(geometry1, geometry2) -> DOUBLE";

/// Minimum distance between two geometries: **metres** on a geographic CRS,
/// native units on a projected one.
///
/// # SQL Signature
/// `ST_DISTANCE(geometry1, geometry2) -> DOUBLE`
///
/// # Behaviour
/// * True shape-to-shape minimum for **every** type pair, `Multi*` and
///   `GeometryCollection` included. Intersecting geometries are 0 apart.
/// * Point-to-point is exact Haversine. Everything else is measured after
///   projecting both operands into one shared metric CRS — see
///   [`super::measure::distance`] for why a projection is unavoidable and what
///   its accuracy costs.
/// * `NULL` in, `NULL` out. Two geometries with *different* explicit SRIDs are an
///   error naming `ST_TRANSFORM`; an unlabelled operand adopts the other's SRID.
///
/// # What changed
/// The previous implementation fell back to **centroid-to-centroid** for
/// Polygon/Polygon and for anything `Multi*`. That reported a positive distance
/// between overlapping polygons and roughly double the true gap between adjacent
/// ones.
///
/// # Examples
/// ```sql
/// SELECT ST_DISTANCE(ST_POINT(-122.4194, 37.7749), ST_POINT(-73.9857, 40.7484));
/// -- ~4129164 metres
///
/// SELECT name FROM shops ORDER BY ST_DISTANCE(location, ST_POINT(8.54, 47.37)) LIMIT 5;
/// ```
pub struct StDistanceFunction;

impl SqlFunction for StDistanceFunction {
    fn name(&self) -> &str {
        "ST_DISTANCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_DISTANCE", SIGNATURE, args, 2)?;
        match geom_pair("ST_DISTANCE", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => Ok(Literal::Double(measure::distance(&a, &b)?)),
        }
    }
}
