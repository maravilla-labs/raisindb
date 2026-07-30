//! ST_Z function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::{expect_arity, geometry_arg};

/// The altitude of a Point, in metres above the WGS84 ellipsoid.
///
/// # Returns
/// * The third ordinate, when the Point has one.
/// * NULL for a two-dimensional Point **and** for any non-Point geometry, which
///   is what PostGIS `ST_Z` does — it returns NULL rather than erroring, so a
///   query over a mixed-dimension column does not blow up on the first flat row.
///
/// # Examples
/// ```sql
/// SELECT ST_Z(location) FROM sensors            -- NULL where 2-D
/// SELECT ST_Z(ST_POINT(8.54, 47.37))            -- NULL
/// ```
///
/// # Notes
/// Altitude is a continuous metric quantity and is **not** a floor or level. A
/// floor is a discrete ordinal label ("L2", "B1"), so it belongs in an ordinary
/// property: two shops on "level 2" of different terminals have different
/// altitudes, and a shop and the void above it share an altitude but not a level.
/// Filtering "same floor" by an altitude band is both wrong and slow.
pub struct StZFunction;

impl SqlFunction for StZFunction {
    fn name(&self) -> &str {
        "ST_Z"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_Z(point) -> DOUBLE"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_Z", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_Z", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        Ok(match raisin_geometry::z_of_point(&geom) {
            Some(z) => Literal::Double(z),
            None => Literal::Null,
        })
    }
}
