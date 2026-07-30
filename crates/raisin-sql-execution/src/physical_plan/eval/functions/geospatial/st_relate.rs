//! ST_RELATE function - the raw DE-9IM matrix, and matching against a pattern

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Expose the DE-9IM intersection matrix that backs all ten topological
/// predicates.
///
/// # SQL Signatures
/// * `ST_RELATE(geometry_a, geometry_b) -> TEXT` — the nine-character matrix
/// * `ST_RELATE(geometry_a, geometry_b, pattern TEXT) -> BOOLEAN` — pattern test
///
/// # Returns
/// * two-argument form: the matrix as nine characters drawn from `F012`, in
///   row-major interior/boundary/exterior order (`II IB IE BI BB BE EI EB EE`)
/// * three-argument form: TRUE if the matrix satisfies `pattern`
/// * NULL if any geometry input is NULL
///
/// # Examples
/// ```sql
/// -- Inspect the exact relationship
/// SELECT ST_RELATE(a.shape, b.shape) FROM parcels a, parcels b;
///
/// -- A relationship none of the named predicates expresses: interiors meet in a
/// -- line, which is how two polygons share a border segment *and* some area.
/// SELECT * FROM parcels p WHERE ST_RELATE(p.shape, $1, '1*1***1**');
/// ```
///
/// # Pattern syntax
/// Exactly nine characters. `0`, `1`, `2` require that dimension; `F` requires
/// emptiness; `T` requires any non-empty dimension; `*` matches anything. A
/// pattern of the wrong length or with an unknown character is a validation
/// error, never a silent FALSE.
///
/// # Notes
/// Planar in the geometry's own coordinate space, like every other topological
/// predicate — see the `relate` module for the geodesic-versus-planar rule.
pub struct StRelateFunction;

impl SqlFunction for StRelateFunction {
    fn name(&self) -> &str {
        "ST_RELATE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_RELATE(geometry_a, geometry_b [, pattern TEXT]) -> TEXT | BOOLEAN"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 && args.len() != 3 {
            return Err(Error::Validation(
                "ST_RELATE requires 2 arguments (returning the DE-9IM matrix) or \
                 3 arguments (matching against a DE-9IM pattern)"
                    .to_string(),
            ));
        }

        // The pattern is read first so that a malformed pattern is reported even
        // when the geometries are NULL: a bad pattern is a query bug, not data.
        let pattern = match args.get(2) {
            None => None,
            Some(expr) => match eval_expr(expr, row)? {
                Literal::Null => return Ok(Literal::Null),
                Literal::Text(s) => {
                    relate::validate_de9im_pattern("ST_RELATE", &s)?;
                    Some(s)
                }
                other => {
                    return Err(Error::Validation(format!(
                        "ST_RELATE pattern must be TEXT, got {:?}",
                        other.data_type()
                    )))
                }
            },
        };

        let pair = relate::resolve_pair("ST_RELATE", &args[0..2], row)?;
        let (a, b) = match pair {
            Some(pair) => pair,
            None => return Ok(Literal::Null),
        };
        let matrix = relate::matrix(&a, &b);

        match pattern {
            None => Ok(Literal::Text(relate::matrix_to_de9im(&matrix))),
            // Already validated above, so this cannot fail; the error is still
            // mapped rather than unwrapped.
            Some(spec) => {
                let matched = matrix.matches(&spec).map_err(|e| {
                    Error::Validation(format!("ST_RELATE: invalid DE-9IM pattern {spec:?}: {e}"))
                })?;
                Ok(Literal::Boolean(matched))
            }
        }
    }
}
