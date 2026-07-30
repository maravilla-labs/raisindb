//! ST_BUFFER - the region within a given distance of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_arg, geom_result};
use super::metric_ops;
use super::z_support::numeric_arg;

const SIGNATURE: &str =
    "ST_BUFFER(geometry, distance) -> GEOMETRY | ST_BUFFER(geometry, distance, quad_segments) -> GEOMETRY";

/// Every point within `distance` of the geometry: **metres** on a geographic CRS,
/// native units on a projected one.
///
/// # SQL Signature
/// `ST_BUFFER(geometry, distance) -> GEOMETRY`
/// `ST_BUFFER(geometry, distance, quad_segments) -> GEOMETRY`
///
/// # What changed
/// The previous implementation collapsed **every** non-Point geometry to its
/// centroid and drew a 32-sided polygon around that. A road's buffer was a disc at
/// its midpoint rather than a corridor along it, and a country's buffer was a
/// circle. This buffers the actual geometry.
///
/// # Behaviour
/// * Works on every geometry type, `Multi*` and `GeometryCollection` included; the
///   result is a `Polygon` or `MultiPolygon`.
/// * A **negative** distance erodes a polygon, as in PostGIS, and may legitimately
///   erode it to nothing — that yields the empty geometry, not an error.
/// * `quad_segments` is the number of straight segments per quarter circle on a
///   round join, so a smaller value gives a coarser, cheaper outline. It must be
///   at least 1.
/// * The CRS and the vertical extent survive.
/// * `NULL` in, `NULL` out.
///
/// # Why the units are honest
/// `geo`'s `Buffer` is planar and works in the geometry's own coordinate units, so
/// on EPSG:4326 a bare `buffer(50)` would mean **fifty degrees** — about 5,500 km.
/// A geographic buffer is therefore projected into a metric CRS, buffered, and
/// projected back. See [`super::metric_ops`].
///
/// # Examples
/// ```sql
/// -- The 500 m catchment of every tram stop.
/// SELECT ST_BUFFER(location, 500) FROM 'stops';
/// ```
pub struct StBufferFunction;

impl SqlFunction for StBufferFunction {
    fn name(&self) -> &str {
        "ST_BUFFER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if !(2..=3).contains(&args.len()) {
            return Err(Error::Validation(format!(
                "ST_BUFFER requires 2 or 3 arguments: {SIGNATURE}"
            )));
        }

        let Some(distance) = numeric_arg("ST_BUFFER", args, 1, row)? else {
            return Ok(Literal::Null);
        };

        let quad_segments = if args.len() == 3 {
            match numeric_arg("ST_BUFFER", args, 2, row)? {
                None => return Ok(Literal::Null),
                Some(n) => Some(n.round() as i64),
            }
        } else {
            None
        };

        match geom_arg("ST_BUFFER", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => geom_result(&metric_ops::buffer(&g, distance, quad_segments)?),
        }
    }
}
