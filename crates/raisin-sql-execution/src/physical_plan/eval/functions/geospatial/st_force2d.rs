//! ST_FORCE2D function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::{expect_arity, geometry_arg};

/// Drop every altitude, returning a strictly two-dimensional geometry.
///
/// Structure-preserving: the geometry type, the ring nesting and every other
/// member (including `srid`) are untouched.
pub struct StForce2DFunction;

impl SqlFunction for StForce2DFunction {
    fn name(&self) -> &str {
        "ST_FORCE2D"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_FORCE2D(geometry) -> GEOMETRY"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_FORCE2D", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_FORCE2D", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        Ok(Literal::Geometry(raisin_geometry::force_2d(&geom)))
    }
}
