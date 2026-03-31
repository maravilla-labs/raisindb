//! TS_RANK function - full-text search ranking

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Get the full-text search ranking score
///
/// # SQL Signature
/// `TS_RANK() -> DOUBLE`
///
/// # Returns
/// * Full-text search ranking score
/// * Error if ts_rank is not available in the current context
///
/// # Examples
/// ```sql
/// SELECT ts_rank FROM documents WHERE to_tsvector(content) @@ to_tsquery('search')
/// ```
///
/// # Notes
/// - ts_rank is a pseudo-column set by the FullTextScan operator
/// - Only available in queries with full-text search conditions
/// - Higher scores indicate better matches
pub struct TsRankFunction;

impl SqlFunction for TsRankFunction {
    fn name(&self) -> &str {
        "TS_RANK"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::FullText
    }

    fn signature(&self) -> &str {
        "TS_RANK() -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, _args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // ts_rank is a pseudo-column set by FullTextScan
        row.get("_ts_rank")
            .and_then(|v| from_property_value(v).ok())
            .ok_or_else(|| Error::Validation("ts_rank not available in this context".to_string()))
    }
}
