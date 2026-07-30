//! ST_ZMAX function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::{expect_arity, geometry_arg};

/// The highest altitude anywhere in a geometry.
///
/// See [`super::st_zmin`]. NULL when the geometry is entirely 2-D.
pub struct StZMaxFunction;

impl SqlFunction for StZMaxFunction {
    fn name(&self) -> &str {
        "ST_ZMAX"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ZMAX(geometry) -> DOUBLE"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ZMAX", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_ZMAX", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        Ok(match raisin_geometry::zdim::z_range_of_json(&geom) {
            Some((_, hi)) => Literal::Double(hi),
            None => Literal::Null,
        })
    }
}
