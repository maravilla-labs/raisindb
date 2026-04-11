//! ST_BUFFER function - create a buffer polygon around a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{get_centroid, get_geometry_type, point_to_geojson};

/// Create a buffer polygon around a geometry at a given distance in meters
///
/// # SQL Signature
/// `ST_BUFFER(geometry, distance_meters) -> GEOMETRY`
pub struct StBufferFunction;

impl SqlFunction for StBufferFunction {
    fn name(&self) -> &str {
        "ST_BUFFER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_BUFFER(geometry, distance_meters) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_BUFFER requires exactly 2 arguments".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;
        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let dist_val = eval_expr(&args[1], row)?;
        if matches!(dist_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_BUFFER requires GEOMETRY as first argument".to_string(),
                ))
            }
        };

        let distance_meters = match &dist_val {
            Literal::Double(d) => *d,
            Literal::Int(i) => *i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_BUFFER requires numeric distance".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        // For Point, create a circle polygon directly
        // For other types, use centroid as center
        let center = if geom_type == "Point" {
            use super::helpers::geojson_to_point;
            geojson_to_point(geom)?
        } else {
            get_centroid(geom)?
        };

        let lon = center.x();
        let lat = center.y();
        let lat_rad = lat.to_radians();
        let meters_per_deg_lat = 110540.0;
        let meters_per_deg_lon = 111320.0 * lat_rad.cos();

        if meters_per_deg_lon.abs() < 1.0 {
            return Err(Error::Validation(
                "ST_BUFFER: cannot create buffer at extreme polar latitudes".to_string(),
            ));
        }

        // Create a 32-sided polygon approximating a circle
        let num_points = 32;
        let mut coords: Vec<Vec<f64>> = Vec::with_capacity(num_points + 1);
        for i in 0..num_points {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (num_points as f64);
            let x = lon + (distance_meters / meters_per_deg_lon) * angle.cos();
            let y = lat + (distance_meters / meters_per_deg_lat) * angle.sin();
            coords.push(vec![x, y]);
        }
        coords.push(coords[0].clone()); // Close the ring

        let result = serde_json::json!({
            "type": "Polygon",
            "coordinates": [coords]
        });

        Ok(Literal::Geometry(result))
    }
}
