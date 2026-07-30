//! ST_ASGEOJSON - serialize a geometry to GeoJSON text.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};
use serde_json::Value;

use super::convert::value_arg;
use super::z_support::numeric_arg;

const SIGNATURE: &str =
    "ST_ASGEOJSON(geometry) -> TEXT | ST_ASGEOJSON(geometry, max_decimals) -> TEXT";

/// A geometry as a GeoJSON string.
///
/// # SQL Signature
/// `ST_ASGEOJSON(geometry) -> TEXT`
/// `ST_ASGEOJSON(geometry, max_decimals) -> TEXT`
///
/// # Behaviour
/// * Serializes the stored representation verbatim, so the **third ordinate
///   survives**. This is the one place altitude must not be dropped, and it is why
///   this function works on the JSON rather than going through `geo` (whose
///   coordinates are strictly 2-D).
/// * A non-4326 geometry keeps its `srid` member — a documented RaisinDB extension,
///   since RFC 7946 mandates WGS84. A 4326 geometry emits no `srid`, so its output
///   is strictly RFC-7946 conformant and drops straight into any mapping library.
/// * `max_decimals` rounds every ordinate, which is the practical way to shrink a
///   tile payload: 5 decimal places is about a metre of longitude, 7 about a
///   centimetre. Omitting it keeps full precision.
/// * `NULL` in, `NULL` out.
pub struct StAsGeoJsonFunction;

impl SqlFunction for StAsGeoJsonFunction {
    fn name(&self) -> &str {
        "ST_ASGEOJSON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if !(1..=2).contains(&args.len()) {
            return Err(Error::Validation(format!(
                "ST_ASGEOJSON requires 1 or 2 arguments: {SIGNATURE}"
            )));
        }

        let max_decimals = if args.len() == 2 {
            match numeric_arg("ST_ASGEOJSON", args, 1, row)? {
                None => return Ok(Literal::Null),
                Some(d) if (0.0..=17.0).contains(&d) => Some(d as u32),
                Some(d) => {
                    return Err(Error::Validation(format!(
                        "ST_ASGEOJSON: max_decimals must be between 0 and 17, got {d}"
                    )))
                }
            }
        } else {
            None
        };

        let Some(mut value) = value_arg("ST_ASGEOJSON", args, 0, row)? else {
            return Ok(Literal::Null);
        };

        if let Some(places) = max_decimals {
            round_in_place(&mut value, places);
        }

        serde_json::to_string(&value)
            .map(Literal::Text)
            .map_err(|e| Error::Validation(format!("ST_ASGEOJSON: cannot serialize geometry: {e}")))
    }
}

/// Round every number in the value tree, however deeply nested.
///
/// Walking the tree rather than only the top-level `coordinates` array is what
/// makes this work for a `MultiPolygon`'s three levels of nesting and for a
/// `GeometryCollection`'s members.
fn round_in_place(value: &mut Value, places: u32) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                let scale = 10f64.powi(places as i32);
                let rounded = (f * scale).round() / scale;
                if let Some(number) = serde_json::Number::from_f64(rounded) {
                    *n = number;
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| round_in_place(i, places)),
        Value::Object(map) => map.values_mut().for_each(|v| round_in_place(v, places)),
        _ => {}
    }
}
