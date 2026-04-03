//! ST_AZIMUTH function - calculate bearing between two points

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::HaversineBearing;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::geojson_to_point;

/// Calculate the azimuth (bearing) between two points
///
/// # SQL Signature
/// `ST_AZIMUTH(point1, point2) -> DOUBLE`
///
/// # Arguments
/// * `point1` - Origin point
/// * `point2` - Destination point
///
/// # Returns
/// * Bearing in radians, normalized to 0..2pi (0 = north, pi/2 = east)
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_AZIMUTH(
///     ST_POINT(-122.4194, 37.7749),
///     ST_POINT(-73.9857, 40.7484)
/// )
/// ```
///
/// # Notes
/// - Uses Haversine bearing formula
/// - Result follows PostGIS convention: 0 = north, pi/2 = east
/// - Normalized to range [0, 2*pi)
pub struct StAzimuthFunction;

impl SqlFunction for StAzimuthFunction {
    fn name(&self) -> &str {
        "ST_AZIMUTH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_AZIMUTH(point1, point2) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_AZIMUTH requires exactly 2 arguments".to_string(),
            ));
        }

        let geom1_val = eval_expr(&args[0], row)?;
        if matches!(geom1_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom2_val = eval_expr(&args[1], row)?;
        if matches!(geom2_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom1 = match &geom1_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_AZIMUTH requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_AZIMUTH requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let point1 = geojson_to_point(geom1)?;
        let point2 = geojson_to_point(geom2)?;

        // haversine_bearing returns degrees (-180..180, 0 = north)
        let bearing_degrees = point1.haversine_bearing(point2);

        // Convert to radians
        let bearing_radians = bearing_degrees.to_radians();

        // Normalize to 0..2*pi (PostGIS convention)
        let two_pi = 2.0 * std::f64::consts::PI;
        let normalized = ((bearing_radians % two_pi) + two_pi) % two_pi;

        Ok(Literal::Double(normalized))
    }
}
