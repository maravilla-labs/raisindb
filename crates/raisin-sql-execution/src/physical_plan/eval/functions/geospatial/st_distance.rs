//! ST_DISTANCE function - calculate distance between geometries

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::compute_haversine_distance;

/// Calculate the geodesic distance between two geometries
///
/// # SQL Signature
/// `ST_DISTANCE(geometry1, geometry2) -> DOUBLE`
///
/// # Arguments
/// * `geometry1` - First geometry
/// * `geometry2` - Second geometry
///
/// # Returns
/// * Distance in meters (using Haversine formula)
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// -- Distance between two points in meters
/// SELECT ST_DISTANCE(
///     ST_POINT(-122.4194, 37.7749),  -- San Francisco
///     ST_POINT(-73.9857, 40.7484)    -- New York
/// )
/// -- Returns: ~4129164.0 (approximately 4129 km)
///
/// -- Distance from a point to a polygon (uses centroid)
/// SELECT ST_DISTANCE(store.location, delivery_zone)
/// FROM stores, zones
/// ```
///
/// # Notes
/// - Uses Haversine formula for accurate geodesic distance
/// - For non-point geometries, uses centroid for distance calculation
/// - Returns distance in meters
pub struct StDistanceFunction;

impl SqlFunction for StDistanceFunction {
    fn name(&self) -> &str {
        "ST_DISTANCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_DISTANCE(geometry1, geometry2) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_DISTANCE requires exactly 2 arguments".to_string(),
            ));
        }

        // Evaluate first geometry
        let geom1_val = eval_expr(&args[0], row)?;
        if matches!(geom1_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Evaluate second geometry
        let geom2_val = eval_expr(&args[1], row)?;
        if matches!(geom2_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Extract GeoJSON values
        let geom1 = match &geom1_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_DISTANCE requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_DISTANCE requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let distance = compute_haversine_distance(geom1, geom2)?;
        Ok(Literal::Double(distance))
    }
}
