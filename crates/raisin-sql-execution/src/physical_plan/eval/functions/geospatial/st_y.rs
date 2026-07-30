//! ST_Y - latitude (or northing) of a point.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::walk::single_point;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_Y(point) -> DOUBLE";

/// The Y ordinate of a point: **latitude** on a geographic CRS, northing on a
/// projected one.
///
/// # SQL Signature
/// `ST_Y(point) -> DOUBLE`
///
/// # Behaviour
/// * Defined for a single location only; anything else gives `NULL`, as in
///   PostGIS. A one-member `MultiPoint` counts as a location.
/// * `NULL` in, `NULL` out.
///
/// For the third ordinate see [`ST_Z`](super::StZFunction), which reads altitude
/// off the GeoJSON representation because `geo`'s coordinates are strictly 2-D.
pub struct StYFunction;

impl SqlFunction for StYFunction {
    fn name(&self) -> &str {
        "ST_Y"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_Y", SIGNATURE, args, 1)?;
        match geom_arg("ST_Y", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(single_point(&g.geometry)
                .map(|p| Literal::Double(p.y()))
                .unwrap_or(Literal::Null)),
        }
    }
}
