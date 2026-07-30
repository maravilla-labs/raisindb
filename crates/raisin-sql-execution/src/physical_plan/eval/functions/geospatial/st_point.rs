//! ST_POINT function - create a point geometry from coordinates

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::axis_guard::check_lon_lat;
use super::helpers::point_to_geojson;

/// Create a Point geometry from longitude and latitude
///
/// # SQL Signature
/// `ST_POINT(longitude, latitude) -> GEOMETRY`
///
/// # Arguments
/// * `longitude` - X coordinate (WGS84, -180 to 180)
/// * `latitude` - Y coordinate (WGS84, -90 to 90)
///
/// # Returns
/// * Point geometry as GeoJSON
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_POINT(-122.4194, 37.7749) -- San Francisco
/// SELECT ST_POINT(2.3522, 48.8566)    -- Paris
/// ```
///
/// # Notes
/// - Coordinates are in WGS84 (EPSG:4326)
/// - **Longitude comes first, then latitude** (GeoJSON / PostGIS convention, not
///   the EPSG authority's lat/lon order for EPSG:4326 — see `axis_guard`)
/// - An unambiguously reversed pair is rejected with a message naming the
///   corrected call; a pair that could be either way round is accepted with a
///   one-time warning, because both ordinates being valid latitudes makes the
///   mistake undetectable
pub struct StPointFunction;

impl SqlFunction for StPointFunction {
    fn name(&self) -> &str {
        "ST_POINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_POINT(longitude, latitude) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_POINT requires exactly 2 arguments (longitude, latitude)".to_string(),
            ));
        }

        // Evaluate longitude
        let lon_val = eval_expr(&args[0], row)?;
        if matches!(lon_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Evaluate latitude
        let lat_val = eval_expr(&args[1], row)?;
        if matches!(lat_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Convert to f64
        let lon = match lon_val {
            Literal::Double(d) => d,
            Literal::Int(i) => i as f64,
            Literal::BigInt(i) => i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_POINT longitude must be numeric".to_string(),
                ))
            }
        };

        let lat = match lat_val {
            Literal::Double(d) => d,
            Literal::Int(i) => i as f64,
            Literal::BigInt(i) => i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_POINT latitude must be numeric".to_string(),
                ))
            }
        };

        // Range validation AND the lon/lat swap heuristic. The old check only
        // caught |lat| > 90, so ST_POINT(47.37, 8.54) — Zurich reversed — passed
        // silently and landed in Somalia.
        check_lon_lat("ST_POINT", lon, lat)?;

        Ok(Literal::Geometry(point_to_geojson(lon, lat)))
    }
}
