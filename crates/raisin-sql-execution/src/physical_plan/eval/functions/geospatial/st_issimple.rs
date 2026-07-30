//! ST_ISSIMPLE - absence of anomalous self-intersection.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::simple;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ISSIMPLE(geometry) -> BOOLEAN";

/// True when a geometry has no self-intersection or self-tangency beyond the
/// vertices consecutive segments necessarily share.
///
/// # SQL Signature
/// `ST_ISSIMPLE(geometry) -> BOOLEAN`
///
/// # What changed
/// The previous implementation returned a constant `true` for every input. This
/// runs a real Bentley-Ottmann sweep (`geo::sweep::Intersections`), in
/// O(n log n).
///
/// # Behaviour by type
/// * **Point** — always simple. **MultiPoint** — simple unless a location repeats.
/// * **LineString** — simple unless it crosses or touches itself. A closed ring's
///   coincident first and last vertex is exempt; a loop returning to an *interior*
///   vertex is not. A spike that doubles back along itself is not simple.
/// * **MultiLineString** — every component simple, and components meeting only at
///   each other's boundary endpoints. Touching the middle of another component is
///   a tangency and is not simple.
/// * **GeometryCollection** — simple only if every member is.
/// * A repeated vertex is tolerated rather than treated as an anomaly.
/// * `NULL` in, `NULL` out.
///
/// # Divergence from PostGIS
/// GEOS — and therefore PostGIS — returns `true` for **every** polygon regardless
/// of its rings, on the grounds that ring quality is
/// [`ST_ISVALID`](super::StIsValidFunction)'s concern. That is defensible under
/// OGC but indistinguishable from the constant-`true` stub this replaces, and it
/// tells a user holding a bow-tie polygon nothing. RaisinDB reports ring
/// simplicity for areal geometry, so a bow-tie polygon is **not** simple here.
pub struct StIsSimpleFunction;

impl SqlFunction for StIsSimpleFunction {
    fn name(&self) -> &str {
        "ST_ISSIMPLE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ISSIMPLE", SIGNATURE, args, 1)?;
        match geom_arg("ST_ISSIMPLE", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Boolean(simple::is_simple(&g.geometry))),
        }
    }
}
