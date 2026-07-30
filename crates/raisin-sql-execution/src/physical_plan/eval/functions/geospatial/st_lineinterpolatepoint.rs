//! ST_LINEINTERPOLATEPOINT - a point a given fraction along a line.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Distance, Euclidean, Geometry, Haversine, Point};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::line_access::sole_line;
use super::z_support::{expect_arity, numeric_arg};

const SIGNATURE: &str = "ST_LINEINTERPOLATEPOINT(linestring, fraction) -> GEOMETRY";

/// The point at `fraction` of the way along a line, measured by **distance**.
///
/// # SQL Signature
/// `ST_LINEINTERPOLATEPOINT(linestring, fraction) -> GEOMETRY`
///
/// # Behaviour
/// * `fraction` is in `[0, 1]`; 0 gives the start point and 1 the end point. A
///   value outside that range is an error rather than a clamp, because a caller
///   passing 1.5 has made an arithmetic mistake and silently returning the endpoint
///   would hide it.
/// * The fraction is of **geodesic** length on a geographic CRS and planar length
///   on a projected one. The previous implementation always measured in raw
///   coordinate units, so at 47°N the halfway point of a diagonal line was placed
///   noticeably off: a degree of longitude there is only about two thirds of a
///   degree of latitude, and the two were weighted equally.
/// * A one-component `MultiLineString` is accepted; anything else gives `NULL`.
/// * A zero-length line returns its own start point.
/// * `NULL` in, `NULL` out. The CRS survives.
pub struct StLineInterpolatePointFunction;

impl SqlFunction for StLineInterpolatePointFunction {
    fn name(&self) -> &str {
        "ST_LINEINTERPOLATEPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_LINEINTERPOLATEPOINT", SIGNATURE, args, 2)?;

        let Some(fraction) = numeric_arg("ST_LINEINTERPOLATEPOINT", args, 1, row)? else {
            return Ok(Literal::Null);
        };
        if !(0.0..=1.0).contains(&fraction) {
            return Err(Error::Validation(format!(
                "ST_LINEINTERPOLATEPOINT: fraction must be between 0.0 and 1.0, got {fraction}"
            )));
        }

        let Some(g) = geom_arg("ST_LINEINTERPOLATEPOINT", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(line) = sole_line(&g) else {
            return Ok(Literal::Null);
        };

        let geographic = g.is_geographic();
        let leg = |a: Point<f64>, b: Point<f64>| {
            if geographic {
                Haversine.distance(a, b)
            } else {
                Euclidean.distance(a, b)
            }
        };

        let vertices: Vec<Point<f64>> = line.0.iter().map(|c| Point::from(*c)).collect();
        let legs: Vec<f64> = vertices
            .windows(2)
            .map(|pair| leg(pair[0], pair[1]))
            .collect();
        let total: f64 = legs.iter().sum();

        if total == 0.0 {
            return derived_result(Geometry::Point(vertices[0]), &g);
        }

        // Walk the legs until the remaining budget falls inside one, then place the
        // point proportionally along that leg. Interpolation within a leg is linear
        // in coordinate space, which is correct to well under a metre for any
        // realistic vertex spacing.
        let mut remaining = fraction * total;
        for (i, leg_length) in legs.iter().enumerate() {
            if remaining <= *leg_length || i == legs.len() - 1 {
                let t = if *leg_length == 0.0 {
                    0.0
                } else {
                    (remaining / leg_length).clamp(0.0, 1.0)
                };
                let (a, b) = (vertices[i], vertices[i + 1]);
                let point = Point::new(a.x() + (b.x() - a.x()) * t, a.y() + (b.y() - a.y()) * t);
                return derived_result(Geometry::Point(point), &g);
            }
            remaining -= leg_length;
        }

        derived_result(Geometry::Point(vertices[vertices.len() - 1]), &g)
    }
}
