//! ST_MAKEENVELOPE - a rectangle from four bounds.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Coord, Geometry, Rect};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_result;
use super::z_support::numeric_arg;
use raisin_geometry::{Crs, Geom};

const SIGNATURE: &str =
    "ST_MAKEENVELOPE(xmin, ymin, xmax, ymax) -> GEOMETRY | ST_MAKEENVELOPE(xmin, ymin, xmax, ymax, srid) -> GEOMETRY";

/// A rectangular Polygon from `(xmin, ymin, xmax, ymax)` — the map-viewport
/// constructor.
///
/// # SQL Signature
/// `ST_MAKEENVELOPE(xmin, ymin, xmax, ymax) -> GEOMETRY`
/// `ST_MAKEENVELOPE(xmin, ymin, xmax, ymax, srid) -> GEOMETRY`
///
/// # Behaviour
/// * Axis order is `(x, y)` = `(longitude, latitude)`, so the argument order is
///   west, south, east, north.
/// * Bounds are normalized: swapped minima and maxima are corrected rather than
///   producing an inverted rectangle, and the ring comes out counter-clockwise as
///   RFC 7946 asks.
/// * The optional fifth argument labels the result's SRID, defaulting to EPSG:4326.
///   This **labels**, it does not reproject — the bounds are interpreted in that
///   CRS.
/// * A degenerate box (zero width or height) still yields a Polygon, so that a
///   viewport query has an areal operand to intersect against.
/// * Any `NULL` argument gives `NULL`.
///
/// # Examples
/// ```sql
/// SELECT * FROM 'shops'
///  WHERE ST_INTERSECTS(location, ST_MAKEENVELOPE(8.50, 47.35, 8.60, 47.40));
/// ```
pub struct StMakeEnvelopeFunction;

impl SqlFunction for StMakeEnvelopeFunction {
    fn name(&self) -> &str {
        "ST_MAKEENVELOPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if !(4..=5).contains(&args.len()) {
            return Err(Error::Validation(format!(
                "ST_MAKEENVELOPE requires 4 or 5 arguments: {SIGNATURE}"
            )));
        }

        let mut bounds = [0.0f64; 4];
        for (i, slot) in bounds.iter_mut().enumerate() {
            match numeric_arg("ST_MAKEENVELOPE", args, i, row)? {
                None => return Ok(Literal::Null),
                Some(v) if v.is_finite() => *slot = v,
                Some(v) => {
                    return Err(Error::Validation(format!(
                        "ST_MAKEENVELOPE: bound {} must be finite, got {v}",
                        i + 1
                    )))
                }
            }
        }

        let srid = if args.len() == 5 {
            match numeric_arg("ST_MAKEENVELOPE", args, 4, row)? {
                None => return Ok(Literal::Null),
                Some(code) => Crs::from_srid(srid_code("ST_MAKEENVELOPE", code)?),
            }
        } else {
            Crs::WGS84
        };

        // `Rect::new` normalizes the corners, and `to_polygon` emits a
        // counter-clockwise closed ring.
        let rect = Rect::new(
            Coord {
                x: bounds[0],
                y: bounds[1],
            },
            Coord {
                x: bounds[2],
                y: bounds[3],
            },
        );
        geom_result(&Geom::new(Geometry::Polygon(rect.to_polygon()), srid))
    }
}

fn srid_code(fn_name: &str, code: f64) -> Result<u32, Error> {
    if code.fract() != 0.0 || code < 1.0 || code > u32::MAX as f64 {
        return Err(Error::Validation(format!(
            "{fn_name}: {code} is not a positive EPSG code"
        )));
    }
    Ok(code as u32)
}
