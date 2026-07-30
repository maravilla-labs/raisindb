//! ST_AREA - area of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::measure;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_AREA(geometry) -> DOUBLE";

/// Area of a geometry: **square metres** on a geographic CRS, square native
/// units on a projected one.
///
/// # SQL Signature
/// `ST_AREA(geometry) -> DOUBLE`
///
/// # Behaviour
/// * Accepts every geometry type. Puntal and linear components contribute 0, so
///   `ST_AREA` of a Point or a LineString is 0 rather than an error.
/// * `MultiPolygon` and `GeometryCollection` sum their areal members, which is
///   what makes `ST_AREA(ST_UNION(a, b))` work when the union yields a
///   MultiPolygon — the case the old Polygon-only implementation rejected.
/// * Interior rings are subtracted.
/// * `NULL` in, `NULL` out.
///
/// # Divergence from PostGIS
/// PostGIS's `geometry` type returns *square degrees* for an EPSG:4326 polygon —
/// a number with no physical meaning that users then scale by a fudge factor.
/// RaisinDB has one geometry type and picks the semantics from the SRID, so a
/// lon/lat area is ellipsoidal square metres (Karney 2013), matching PostGIS's
/// `geography` type. On a projected CRS the planar area is returned in that CRS's
/// units, exactly as PostGIS does.
pub struct StAreaFunction;

impl SqlFunction for StAreaFunction {
    fn name(&self) -> &str {
        "ST_AREA"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_AREA", SIGNATURE, args, 1)?;
        match geom_arg("ST_AREA", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Double(measure::area(&g))),
        }
    }
}
