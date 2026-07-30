//! ST_ISCLOSED - whether every linear component starts where it ends.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::walk::for_each_line_string;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ISCLOSED(geometry) -> BOOLEAN";

/// True when every linear component of a geometry has coincident start and end
/// points.
///
/// # SQL Signature
/// `ST_ISCLOSED(geometry) -> BOOLEAN`
///
/// # Behaviour
/// * `LineString` — closed when its first and last vertex coincide.
/// * `MultiLineString` — closed only when **every** component is; previously this
///   was an error. A single open component makes the whole geometry open.
/// * Puntal geometries are closed by definition (they have no boundary), and areal
///   geometries are closed because polygon rings always are. Both match PostGIS.
/// * `GeometryCollection` — closed when all of its linear members are.
/// * A LineString with fewer than two vertices is **not** closed: there is no
///   circuit to complete.
/// * `NULL` in, `NULL` out.
pub struct StIsClosedFunction;

impl SqlFunction for StIsClosedFunction {
    fn name(&self) -> &str {
        "ST_ISCLOSED"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ISCLOSED", SIGNATURE, args, 1)?;
        match geom_arg("ST_ISCLOSED", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Boolean(all_linear_components_closed(&g.geometry))),
        }
    }
}

fn all_linear_components_closed(g: &Geometry<f64>) -> bool {
    let mut closed = true;
    for_each_line_string(g, &mut |ls| {
        if ls.0.len() < 2 || !ls.is_closed() {
            closed = false;
        }
    });
    closed
}
