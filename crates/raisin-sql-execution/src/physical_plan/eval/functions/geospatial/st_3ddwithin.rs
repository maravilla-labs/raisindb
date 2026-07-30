//! ST_3DDWITHIN function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::st_3ddistance::distance_3d;
use super::z_support::numeric_arg;
use super::z_support::{expect_arity, geometry_arg};

/// True when two geometries are within `distance` metres of each other in three
/// dimensions.
///
/// Equivalent to `ST_3DDISTANCE(a, b) <= distance`, and NULL under the same
/// condition (either operand two-dimensional).
///
/// # Notes
/// This has **no index support**: the spatial index is keyed on 2-D geohash cells.
/// Combine it with a 2-D `ST_DWITHIN` on the same radius to get an index-driven
/// candidate set that this then narrows — the horizontal distance is a lower bound
/// on the 3-D distance, so that composition never drops a matching row.
pub struct St3DDWithinFunction;

impl SqlFunction for St3DDWithinFunction {
    fn name(&self) -> &str {
        "ST_3DDWITHIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_3DDWITHIN(geometry1, geometry2, distance) -> BOOLEAN"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_3DDWITHIN", self.signature(), args, 3)?;
        let Some(a) = geometry_arg("ST_3DDWITHIN", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(b) = geometry_arg("ST_3DDWITHIN", args, 1, row)? else {
            return Ok(Literal::Null);
        };
        let Some(max) = numeric_arg("ST_3DDWITHIN", args, 2, row)? else {
            return Ok(Literal::Null);
        };
        if max < 0.0 {
            return Err(Error::Validation(
                "ST_3DDWITHIN: distance must be non-negative".to_string(),
            ));
        }
        match distance_3d(&a, &b)? {
            Some(d) => Ok(Literal::Boolean(d <= max)),
            None => Ok(Literal::Null),
        }
    }
}
