//! ST_DWITHIN - proximity test, and the predicate the spatial index answers.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_pair;
use super::measure;
use super::z_support::{expect_arity, numeric_arg};

const SIGNATURE: &str = "ST_DWITHIN(geometry1, geometry2, distance) -> BOOLEAN";

/// True when two geometries are no more than `distance` apart: **metres** on a
/// geographic CRS, native units on a projected one.
///
/// # SQL Signature
/// `ST_DWITHIN(geometry1, geometry2, distance) -> BOOLEAN`
///
/// # Behaviour
/// * Exactly `ST_DISTANCE(a, b) <= distance`, so it inherits true shape-to-shape
///   distance for every type pair — including the `Multi*` and Polygon/Polygon
///   cases that previously went through a centroid approximation and therefore
///   answered the wrong question near the boundary.
/// * A negative distance is rejected rather than silently answering `false`.
/// * `NULL` in any argument gives `NULL`.
///
/// # Why this one matters most
/// This is the predicate the spatial index is built to answer. The planner
/// rewrites `ST_DWITHIN(<geometry column>, <constant point>, <constant radius>)`
/// into an index scan; the row-level implementation here is what runs when the
/// index cannot be used, and it must agree with the index's own Haversine
/// post-filter to the metre — which it does, because both use the GRS80 mean
/// radius from `raisin_geometry::EARTH_MEAN_RADIUS_METERS`.
///
/// # Examples
/// ```sql
/// SELECT * FROM 'shops' WHERE ST_DWITHIN(location, ST_POINT(8.54, 47.37), 500);
/// ```
pub struct StDWithinFunction;

impl SqlFunction for StDWithinFunction {
    fn name(&self) -> &str {
        "ST_DWITHIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_DWITHIN", SIGNATURE, args, 3)?;

        let Some(max_distance) = numeric_arg("ST_DWITHIN", args, 2, row)? else {
            return Ok(Literal::Null);
        };
        if max_distance < 0.0 || !max_distance.is_finite() {
            return Err(Error::Validation(format!(
                "ST_DWITHIN: distance must be a non-negative finite number, got {max_distance}"
            )));
        }

        match geom_pair("ST_DWITHIN", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => Ok(Literal::Boolean(measure::distance(&a, &b)? <= max_distance)),
        }
    }
}
