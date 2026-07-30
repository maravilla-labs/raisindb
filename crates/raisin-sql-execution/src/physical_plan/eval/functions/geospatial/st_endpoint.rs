//! ST_ENDPOINT - last vertex of a linear geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::line_access::sole_line;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ENDPOINT(geometry) -> GEOMETRY";

/// The last vertex of a linear geometry, as a Point.
///
/// # SQL Signature
/// `ST_ENDPOINT(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * A one-component `MultiLineString` is accepted; two or more components, or a
///   non-linear geometry, give `NULL` as in PostGIS.
/// * `NULL` in, `NULL` out. The CRS survives.
///
/// Together with [`ST_STARTPOINT`](super::StStartPointFunction) this is the pair
/// [`ST_BOUNDARY`](super::StBoundaryFunction) returns for an open line — and
/// `ST_BOUNDARY` correctly returns *nothing* for a closed one, whereas these two
/// still report its coincident seam.
pub struct StEndPointFunction;

impl SqlFunction for StEndPointFunction {
    fn name(&self) -> &str {
        "ST_ENDPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ENDPOINT", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_ENDPOINT", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        match sole_line(&g) {
            None => Ok(Literal::Null),
            Some(line) => {
                let last = line.0[line.0.len() - 1];
                derived_result(Geometry::Point(last.into()), &g)
            }
        }
    }
}
