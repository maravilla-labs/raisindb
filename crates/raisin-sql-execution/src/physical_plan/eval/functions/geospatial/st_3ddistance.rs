//! ST_3DDISTANCE function.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::compute_haversine_distance;
use super::z_support::{expect_arity, geometry_arg};
use raisin_geometry::zdim::{z_gap, z_range_of_json};

/// `hypot(horizontal, vertical)`; `None` when either side is 2-D.
///
/// Shared with `ST_3DDWITHIN` so the two can never disagree about what "3-D
/// distance" means.
pub(super) fn distance_3d(
    a: &serde_json::Value,
    b: &serde_json::Value,
) -> Result<Option<f64>, Error> {
    let Some(dz) = z_gap(z_range_of_json(a), z_range_of_json(b)) else {
        return Ok(None);
    };
    let horizontal = compute_haversine_distance(a, b)?;
    Ok(Some(horizontal.hypot(dz)))
}

/// Distance between two geometries including the vertical component, in metres.
///
/// NULL when either operand is two-dimensional: there is no third coordinate to
/// measure against, and inventing zero would silently answer a different
/// question than the one asked.
///
/// # Divergence from PostGIS, deliberately
///
/// PostGIS `ST_3DDistance` is fully Cartesian in the geometry's own CRS and
/// refuses `geography`. Ours is **geodesic horizontally, Euclidean vertically** —
/// `hypot(ST_DISTANCE, dz)` — because that is the only defensible answer when the
/// horizontal ordinates are lon/lat degrees and the vertical one is metres above
/// the ellipsoid. Mixing those in one Cartesian formula would produce a number
/// with no unit at all.
///
/// The vertical leg is the **gap between the two altitude intervals**, which is
/// zero when they overlap. So a point inside a tall building's altitude band is
/// vertically coincident with it, which is the useful answer for "how far apart
/// are these things".
pub struct St3DDistanceFunction;

impl SqlFunction for St3DDistanceFunction {
    fn name(&self) -> &str {
        "ST_3DDISTANCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_3DDISTANCE(geometry1, geometry2) -> DOUBLE"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_3DDISTANCE", self.signature(), args, 2)?;
        let Some(a) = geometry_arg("ST_3DDISTANCE", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(b) = geometry_arg("ST_3DDISTANCE", args, 1, row)? else {
            return Ok(Literal::Null);
        };
        match distance_3d(&a, &b)? {
            Some(d) => Ok(Literal::Double(d)),
            None => Ok(Literal::Null),
        }
    }
}
