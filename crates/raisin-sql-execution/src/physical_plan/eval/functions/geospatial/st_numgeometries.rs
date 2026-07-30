//! ST_NUMGEOMETRIES - component count of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_NUMGEOMETRIES(geometry) -> INTEGER";

/// The number of components in a geometry.
///
/// # SQL Signature
/// `ST_NUMGEOMETRIES(geometry) -> INTEGER`
///
/// # Behaviour
/// * 1 for a single geometry (Point, LineString, Polygon), the member count for a
///   `Multi*` or a `GeometryCollection`, and 0 for the empty geometry.
/// * Counts only the **top level**: a GeometryCollection holding one MultiPolygon
///   of three polygons has 1 geometry, not 3. Use `ST_NUMPOINTS` for a total or
///   `ST_GEOMETRYN` to descend.
/// * `NULL` in, `NULL` out.
pub struct StNumGeometriesFunction;

impl SqlFunction for StNumGeometriesFunction {
    fn name(&self) -> &str {
        "ST_NUMGEOMETRIES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_NUMGEOMETRIES", SIGNATURE, args, 1)?;
        match geom_arg("ST_NUMGEOMETRIES", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Int(component_count(&g.geometry))),
        }
    }
}

fn component_count(g: &Geometry<f64>) -> i32 {
    let count = match g {
        Geometry::MultiPoint(mp) => mp.0.len(),
        Geometry::MultiLineString(mls) => mls.0.len(),
        Geometry::MultiPolygon(mp) => mp.0.len(),
        Geometry::GeometryCollection(gc) => gc.0.len(),
        _ => 1,
    };
    count.try_into().unwrap_or(i32::MAX)
}
