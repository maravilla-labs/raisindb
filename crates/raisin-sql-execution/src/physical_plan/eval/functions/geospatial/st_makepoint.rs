//! ST_MAKEPOINT function - create a point geometry from coordinates (PostGIS alias)

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::point_to_geojson;

/// Create a Point geometry from X and Y coordinates
///
/// # SQL Signature
/// `ST_MAKEPOINT(x, y) -> GEOMETRY`
///
/// # Arguments
/// * `x` - X coordinate (longitude in WGS84, -180 to 180)
/// * `y` - Y coordinate (latitude in WGS84, -90 to 90)
///
/// # Returns
/// * Point geometry as GeoJSON
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_MAKEPOINT(-122.4194, 37.7749)
/// ```
///
/// # Notes
/// - PostGIS-compatible alias for ST_POINT
/// - Coordinates are in WGS84 (EPSG:4326)
pub struct StMakePointFunction;

impl SqlFunction for StMakePointFunction {
    fn name(&self) -> &str {
        "ST_MAKEPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_MAKEPOINT(x, y) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_MAKEPOINT requires exactly 2 arguments (x, y)".to_string(),
            ));
        }

        let lon_val = eval_expr(&args[0], row)?;
        if matches!(lon_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let lat_val = eval_expr(&args[1], row)?;
        if matches!(lat_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let lon = match lon_val {
            Literal::Double(d) => d,
            Literal::Int(i) => i as f64,
            Literal::BigInt(i) => i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_MAKEPOINT x must be numeric".to_string(),
                ))
            }
        };

        let lat = match lat_val {
            Literal::Double(d) => d,
            Literal::Int(i) => i as f64,
            Literal::BigInt(i) => i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_MAKEPOINT y must be numeric".to_string(),
                ))
            }
        };

        if !(-180.0..=180.0).contains(&lon) {
            return Err(Error::Validation(format!(
                "X coordinate {} out of range [-180, 180]",
                lon
            )));
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err(Error::Validation(format!(
                "Y coordinate {} out of range [-90, 90]",
                lat
            )));
        }

        Ok(Literal::Geometry(point_to_geojson(lon, lat)))
    }
}
