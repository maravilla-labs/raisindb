//! ST_ZMIN function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::{expect_arity, geometry_arg};

/// The lowest altitude anywhere in a geometry.
///
/// Reads the `(min, max)` vertical extent, so unlike `ST_Z` it works for every
/// geometry type, not just Point. NULL when the geometry is entirely 2-D.
pub struct StZMinFunction;

impl SqlFunction for StZMinFunction {
    fn name(&self) -> &str {
        "ST_ZMIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ZMIN(geometry) -> DOUBLE"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ZMIN", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_ZMIN", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        Ok(match raisin_geometry::zdim::z_range_of_json(&geom) {
            Some((lo, _)) => Literal::Double(lo),
            None => Literal::Null,
        })
    }
}
