//! ST_ASGEOJSON function - convert geometry to GeoJSON text

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Convert a geometry to GeoJSON text
///
/// # SQL Signature
/// `ST_ASGEOJSON(geometry) -> TEXT`
///
/// # Arguments
/// * `geometry` - A geometry value
///
/// # Returns
/// * GeoJSON string representation
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_ASGEOJSON(location) FROM stores
/// SELECT ST_ASGEOJSON(ST_POINT(-122.4194, 37.7749))
/// -- Returns: '{"type":"Point","coordinates":[-122.4194,37.7749]}'
/// ```
pub struct StAsGeoJsonFunction;

impl SqlFunction for StAsGeoJsonFunction {
    fn name(&self) -> &str {
        "ST_ASGEOJSON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ASGEOJSON(geometry) -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ASGEOJSON requires exactly 1 argument".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;

        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        match geom_val {
            Literal::Geometry(v) => {
                let json_str = serde_json::to_string(&v).map_err(|e| {
                    Error::Validation(format!("Failed to serialize geometry: {}", e))
                })?;
                Ok(Literal::Text(json_str))
            }
            Literal::JsonB(v) => {
                // Accept JSONB as well (might be geometry stored as JSONB)
                let json_str = serde_json::to_string(&v).map_err(|e| {
                    Error::Validation(format!("Failed to serialize geometry: {}", e))
                })?;
                Ok(Literal::Text(json_str))
            }
            _ => Err(Error::Validation(
                "ST_ASGEOJSON requires GEOMETRY input".to_string(),
            )),
        }
    }
}
