//! ST_UNION function - compute the union of two geometries

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::BooleanOps;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_point, geojson_to_polygon, get_geometry_type, multipoint_to_geojson,
    polygon_to_geojson,
};

/// Compute the union of two geometries
///
/// # SQL Signature
/// `ST_UNION(geometry1, geometry2) -> GEOMETRY`
pub struct StUnionFunction;

impl SqlFunction for StUnionFunction {
    fn name(&self) -> &str {
        "ST_UNION"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_UNION(geometry1, geometry2) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_UNION requires exactly 2 arguments".to_string(),
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
                    "ST_UNION requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_UNION requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type1 = get_geometry_type(geom1)?;
        let type2 = get_geometry_type(geom2)?;

        match (type1, type2) {
            ("Polygon", "Polygon") => {
                let poly1 = geojson_to_polygon(geom1)?;
                let poly2 = geojson_to_polygon(geom2)?;
                let result = poly1.union(&poly2);
                // BooleanOps returns a MultiPolygon
                let polys: Vec<serde_json::Value> =
                    result.0.iter().map(|p| polygon_to_geojson(p)).collect();
                if polys.len() == 1 {
                    Ok(Literal::Geometry(polys.into_iter().next().unwrap()))
                } else {
                    let coords: Vec<serde_json::Value> = result
                        .0
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
            ("Point", "Point") => {
                let p1 = geojson_to_point(geom1)?;
                let p2 = geojson_to_point(geom2)?;
                let result = multipoint_to_geojson(&[p1, p2]);
                Ok(Literal::Geometry(result))
            }
            _ => Err(Error::Validation(format!(
                "ST_UNION not supported for {} and {}. Supports: Polygon+Polygon, Point+Point",
                type1, type2
            ))),
        }
    }
}
