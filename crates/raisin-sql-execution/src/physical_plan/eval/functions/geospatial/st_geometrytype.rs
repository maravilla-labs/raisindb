//! ST_GEOMETRYTYPE - the type name of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_GEOMETRYTYPE(geometry) -> TEXT";

/// The geometry's type as `ST_Point`, `ST_MultiPolygon` and so on.
///
/// # SQL Signature
/// `ST_GEOMETRYTYPE(geometry) -> TEXT`
///
/// # Behaviour
/// * The `ST_` prefix is PostGIS's convention for this function, kept so that
///   existing `WHERE ST_GEOMETRYTYPE(g) = 'ST_Point'` filters port unchanged.
/// * Reported after parsing rather than by echoing the JSON's `type` member, so a
///   malformed geometry is a query error instead of an invented type name.
/// * `NULL` in, `NULL` out.
pub struct StGeometryTypeFunction;

impl SqlFunction for StGeometryTypeFunction {
    fn name(&self) -> &str {
        "ST_GEOMETRYTYPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_GEOMETRYTYPE", SIGNATURE, args, 1)?;
        match geom_arg("ST_GEOMETRYTYPE", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Text(format!("ST_{}", type_name(&g.geometry)))),
        }
    }
}

/// The GeoJSON spelling of a `geo` geometry's type.
///
/// `Line`, `Rect` and `Triangle` are `geo`-only shapes with no GeoJSON name, so
/// they are reported as what they serialize to.
fn type_name(g: &Geometry<f64>) -> &'static str {
    match g {
        Geometry::Point(_) => "Point",
        Geometry::MultiPoint(_) => "MultiPoint",
        Geometry::Line(_) | Geometry::LineString(_) => "LineString",
        Geometry::MultiLineString(_) => "MultiLineString",
        Geometry::Polygon(_) | Geometry::Rect(_) | Geometry::Triangle(_) => "Polygon",
        Geometry::MultiPolygon(_) => "MultiPolygon",
        Geometry::GeometryCollection(_) => "GeometryCollection",
    }
}
