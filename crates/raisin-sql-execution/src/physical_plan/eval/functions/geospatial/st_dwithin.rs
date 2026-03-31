//! ST_DWITHIN function - check if geometries are within distance

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::HaversineDistance;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, get_centroid, get_geometry_type};

/// Check if two geometries are within a specified distance
///
/// # SQL Signature
/// `ST_DWITHIN(geometry1, geometry2, distance_meters) -> BOOLEAN`
///
/// # Arguments
/// * `geometry1` - First geometry
/// * `geometry2` - Second geometry
/// * `distance_meters` - Maximum distance in meters
///
/// # Returns
/// * TRUE if distance <= threshold
/// * FALSE if distance > threshold
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// -- Find stores within 5km of user location
/// SELECT * FROM stores
/// WHERE ST_DWITHIN(location, ST_POINT(-122.4194, 37.7749), 5000)
///
/// -- Find nearby restaurants
/// SELECT name FROM restaurants
/// WHERE ST_DWITHIN(
///     location,
///     ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[-73.9857,40.7484]}'),
///     1000  -- 1km radius
/// )
/// ```
///
/// # Notes
/// - This is the primary function for spatial proximity queries
/// - Uses Haversine formula for accurate geodesic distance
/// - More efficient than ST_DISTANCE() < threshold for indexed queries
pub struct StDWithinFunction;

impl SqlFunction for StDWithinFunction {
    fn name(&self) -> &str {
        "ST_DWITHIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_DWITHIN(geometry1, geometry2, distance_meters) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 3 {
            return Err(Error::Validation(
                "ST_DWITHIN requires exactly 3 arguments (geom1, geom2, distance)".to_string(),
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

        // Evaluate distance threshold
        let distance_val = eval_expr(&args[2], row)?;
        if matches!(distance_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let max_distance = match distance_val {
            Literal::Double(d) => d,
            Literal::Int(i) => i as f64,
            Literal::BigInt(i) => i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_DWITHIN distance must be numeric".to_string(),
                ))
            }
        };

        if max_distance < 0.0 {
            return Err(Error::Validation(
                "ST_DWITHIN distance must be non-negative".to_string(),
            ));
        }

        // Extract GeoJSON values
        let geom1 = match &geom1_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_DWITHIN requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_DWITHIN requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        // Get points for distance calculation
        let point1 = if get_geometry_type(geom1)? == "Point" {
            geojson_to_point(geom1)?
        } else {
            get_centroid(geom1)?
        };

        let point2 = if get_geometry_type(geom2)? == "Point" {
            geojson_to_point(geom2)?
        } else {
            get_centroid(geom2)?
        };

        // Calculate Haversine distance
        let distance = point1.haversine_distance(&point2);

        Ok(Literal::Boolean(distance <= max_distance))
    }
}
