//! ST_MAKEENVELOPE function - create a rectangular Polygon from bounds

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Create a rectangular Polygon from bounding box coordinates
///
/// # SQL Signature
/// `ST_MAKEENVELOPE(xmin, ymin, xmax, ymax) -> GEOMETRY`
pub struct StMakeEnvelopeFunction;

impl SqlFunction for StMakeEnvelopeFunction {
    fn name(&self) -> &str {
        "ST_MAKEENVELOPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_MAKEENVELOPE(xmin, ymin, xmax, ymax) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 4 {
            return Err(Error::Validation(
                "ST_MAKEENVELOPE requires exactly 4 arguments".to_string(),
            ));
        }

        let mut vals = Vec::with_capacity(4);
        for (i, arg) in args.iter().enumerate() {
            let val = eval_expr(arg, row)?;
            if matches!(val, Literal::Null) {
                return Ok(Literal::Null);
            }
            match &val {
                Literal::Double(d) => vals.push(*d),
                Literal::Int(n) => vals.push(*n as f64),
                _ => {
                    return Err(Error::Validation(format!(
                        "ST_MAKEENVELOPE argument {} must be numeric",
                        i + 1
                    )))
                }
            }
        }

        let xmin = vals[0];
        let ymin = vals[1];
        let xmax = vals[2];
        let ymax = vals[3];

        let result = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[
                [xmin, ymin],
                [xmax, ymin],
                [xmax, ymax],
                [xmin, ymax],
                [xmin, ymin]
            ]]
        });

        Ok(Literal::Geometry(result))
    }
}
