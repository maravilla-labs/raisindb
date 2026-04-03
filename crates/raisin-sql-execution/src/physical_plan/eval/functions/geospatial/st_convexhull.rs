//! ST_CONVEXHULL function - return the convex hull of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::ConvexHull;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_polygon, get_geometry_type, point_to_geojson,
    polygon_to_geojson,
};

/// Return the convex hull of a geometry as a Polygon
///
/// # SQL Signature
/// `ST_CONVEXHULL(geometry) -> GEOMETRY`
pub struct StConvexHullFunction;

impl SqlFunction for StConvexHullFunction {
    fn name(&self) -> &str {
        "ST_CONVEXHULL"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CONVEXHULL(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_CONVEXHULL requires exactly 1 argument".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;
        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_CONVEXHULL requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" => {
                // Convex hull of a point is the point itself
                Ok(Literal::Geometry(geom.clone()))
            }
            "LineString" => {
                let line = geojson_to_linestring(geom)?;
                let hull = line.convex_hull();
                let result = polygon_to_geojson(&hull);
                Ok(Literal::Geometry(result))
            }
            "Polygon" => {
                let polygon = geojson_to_polygon(geom)?;
                let hull = polygon.convex_hull();
                let result = polygon_to_geojson(&hull);
                Ok(Literal::Geometry(result))
            }
            other => Err(Error::Validation(format!(
                "ST_CONVEXHULL not supported for geometry type: {}",
                other
            ))),
        }
    }
}
