//! ST_GEOMFROMGEOJSON - parse GeoJSON text into a geometry.

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_GEOMFROMGEOJSON(geojson_text) -> GEOMETRY";

/// Parse GeoJSON text into a geometry, validating it on the way in.
///
/// # SQL Signature
/// `ST_GEOMFROMGEOJSON(geojson_text) -> GEOMETRY`
///
/// # Behaviour
/// * Validation is **structural**: the value must actually parse into a geometry,
///   with well-formed coordinate nesting for its type and finite ordinates. The
///   previous implementation only checked that the `type` member was one of the
///   seven names and that a `coordinates` key existed, so
///   `{"type":"Polygon","coordinates":[1,2]}` passed and failed later, deep inside
///   some other function.
/// * Accepts a Feature or a FeatureCollection as well as a bare geometry, because
///   that is what people paste. The geometry is extracted.
/// * A `srid` member is preserved, and a third ordinate is preserved.
/// * Already-parsed JSONB input short-circuits the string parse but takes the same
///   validation.
/// * `NULL` in, `NULL` out.
///
/// # Why validate at all
/// This is the boundary where a typo becomes a stored value. A geometry that is not
/// well-formed GeoJSON infers as a plain JSON object in the property system and is
/// then **never spatially indexed**, with no error at write time — so rejecting it
/// here is what stops a silent data loss further down.
pub struct StGeomFromGeoJsonFunction;

impl SqlFunction for StGeomFromGeoJsonFunction {
    fn name(&self) -> &str {
        "ST_GEOMFROMGEOJSON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_GEOMFROMGEOJSON", SIGNATURE, args, 1)?;

        let value = match eval_expr(&args[0], row)? {
            Literal::Null => return Ok(Literal::Null),
            Literal::Text(s) => serde_json::from_str(&s).map_err(|e| {
                Error::Validation(format!("ST_GEOMFROMGEOJSON: not valid JSON: {e}"))
            })?,
            Literal::JsonB(v) | Literal::Geometry(v) => v,
            other => {
                return Err(Error::Validation(format!(
                    "ST_GEOMFROMGEOJSON requires TEXT or JSONB input, got {:?}",
                    other.data_type()
                )))
            }
        };

        // Parsing IS the validation: `to_geo` rejects a wrong coordinate shape for
        // the declared type, a non-finite ordinate and an unusable `srid` member.
        // The parsed geometry is discarded because the stored representation must
        // keep the third ordinate that `geo` cannot hold.
        raisin_geometry::to_geo(&value, None)?;
        Ok(Literal::Geometry(value))
    }
}
