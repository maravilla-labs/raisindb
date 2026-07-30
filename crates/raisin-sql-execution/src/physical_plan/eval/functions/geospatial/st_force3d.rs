//! ST_FORCE3D function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::numeric_arg;
use super::z_support::{expect_arity, geometry_arg};

/// Give every two-dimensional position the supplied altitude.
///
/// Positions that already carry a Z keep it, matching PostGIS `ST_Force3D`, which
/// fills the missing ordinate rather than overwriting a present one. Use
/// `ST_FORCE2D` first to overwrite unconditionally.
pub struct StForce3DFunction;

impl SqlFunction for StForce3DFunction {
    fn name(&self) -> &str {
        "ST_FORCE3D"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_FORCE3D(geometry, z) -> GEOMETRY"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_FORCE3D", self.signature(), args, 2)?;
        let Some(geom) = geometry_arg("ST_FORCE3D", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(z) = numeric_arg("ST_FORCE3D", args, 1, row)? else {
            return Ok(Literal::Null);
        };
        if !z.is_finite() {
            return Err(Error::Validation(
                "ST_FORCE3D: z must be a finite number".to_string(),
            ));
        }
        Ok(Literal::Geometry(raisin_geometry::force_3d(&geom, z)))
    }
}
