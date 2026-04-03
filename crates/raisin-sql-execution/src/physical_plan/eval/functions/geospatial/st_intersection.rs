//! ST_INTERSECTION function - compute the intersection of two geometries

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::BooleanOps;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_polygon, get_geometry_type, polygon_to_geojson};

/// Compute the intersection of two geometries
///
/// # SQL Signature
/// `ST_INTERSECTION(geometry1, geometry2) -> GEOMETRY`
pub struct StIntersectionFunction;

impl SqlFunction for StIntersectionFunction {
    fn name(&self) -> &str {
        "ST_INTERSECTION"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_INTERSECTION(geometry1, geometry2) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_INTERSECTION requires exactly 2 arguments".to_string(),
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
                    "ST_INTERSECTION requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_INTERSECTION requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type1 = get_geometry_type(geom1)?;
        let type2 = get_geometry_type(geom2)?;

        match (type1, type2) {
            ("Polygon", "Polygon") => {
                let poly1 = geojson_to_polygon(geom1)?;
                let poly2 = geojson_to_polygon(geom2)?;
                let result = poly1.intersection(&poly2);
                let polys: Vec<&geo::Polygon> = result.0.iter().collect();
                if polys.is_empty() {
                    Ok(Literal::Geometry(serde_json::json!({
                        "type": "GeometryCollection",
                        "geometries": []
                    })))
                } else if polys.len() == 1 {
                    Ok(Literal::Geometry(polygon_to_geojson(polys[0])))
                } else {
                    let coords: Vec<serde_json::Value> = polys
                        .iter()
                        .map(|p| {
                            let exterior: Vec<Vec<f64>> =
                                p.exterior().coords().map(|c| vec![c.x, c.y]).collect();
                            let mut rings = vec![exterior];
                            for interior in p.interiors() {
                                let ring: Vec<Vec<f64>> =
                                    interior.coords().map(|c| vec![c.x, c.y]).collect();
                                rings.push(ring);
                            }
                            serde_json::json!(rings)
                        })
                        .collect();
                    Ok(Literal::Geometry(serde_json::json!({
                        "type": "MultiPolygon",
                        "coordinates": coords
                    })))
                }
            }
            _ => Err(Error::Validation(format!(
                "ST_INTERSECTION not supported for {} and {}. Supports: Polygon+Polygon",
                type1, type2
            ))),
        }
    }
}
