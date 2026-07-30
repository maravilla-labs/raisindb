//! ST_NDIMS function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::{expect_arity, geometry_arg};

/// The coordinate dimension of a geometry: 2 or 3.
///
/// 3 when *any* position carries an altitude, so a geometry with a mix of 2-D and
/// 3-D vertices reports 3. That matches how the vertical extent is computed and
/// keeps `ST_NDIMS(g) = 3` a reliable predicate for "this row has altitude data".
pub struct StNDimsFunction;

impl SqlFunction for StNDimsFunction {
    fn name(&self) -> &str {
        "ST_NDIMS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_NDIMS(geometry) -> INTEGER"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_NDIMS", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_NDIMS", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        Ok(Literal::Int(raisin_geometry::ndims(&geom) as i32))
    }
}
