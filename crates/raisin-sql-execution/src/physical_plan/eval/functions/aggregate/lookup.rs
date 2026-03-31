//! Aggregate function lookup implementations
//!
//! Aggregate functions (COUNT, SUM, AVG, MIN, MAX, ARRAY_AGG) are pre-computed
//! by the HashAggregate operator. This module provides function implementations
//! that look up the pre-computed values from the row.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::naming::generate_aggregate_column_name;

/// Base implementation for aggregate lookup functions
///
/// All aggregate functions share the same lookup logic - they find their
/// pre-computed value in the row using a canonical column name.
struct AggregateLookupFunction {
    name: &'static str,
}

impl AggregateLookupFunction {
    const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

impl SqlFunction for AggregateLookupFunction {
    fn name(&self) -> &str {
        self.name
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }

    fn signature(&self) -> &str {
        match self.name {
            "COUNT" => "COUNT(expr?) -> INT",
            "SUM" => "SUM(numeric) -> NUMERIC",
            "AVG" => "AVG(numeric) -> DOUBLE",
            "MIN" => "MIN(expr) -> ANY",
            "MAX" => "MAX(expr) -> ANY",
            "ARRAY_AGG" => "ARRAY_AGG(expr) -> ARRAY",
            _ => "AGGREGATE(expr) -> ANY",
        }
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Generate canonical name for this aggregate
        let canonical_name = generate_aggregate_column_name(self.name, args);

        tracing::debug!(
            "Aggregate {}: Looking for canonical name '{}'. Row keys: {:?}",
            self.name,
            canonical_name,
            row.columns.keys().collect::<Vec<_>>()
        );

        // Try to find it in the row
        if let Some(value) = row.get(&canonical_name) {
            tracing::debug!(
                "✓ Found aggregate value for '{}': {:?}",
                canonical_name,
                value
            );
            from_property_value(value).map_err(Error::Backend)
        } else {
            tracing::error!(
                "✗ Aggregate {} with canonical name '{}' not found in row. Available keys: {:?}",
                self.name,
                canonical_name,
                row.columns.keys().collect::<Vec<_>>()
            );
            Err(Error::Validation(format!(
                "Aggregate function {} result not found in row. This may indicate the query needs a GROUP BY clause or the aggregate wasn't computed correctly.",
                self.name
            )))
        }
    }
}

/// COUNT aggregate function lookup
pub struct CountFunction;
impl SqlFunction for CountFunction {
    fn name(&self) -> &str {
        "COUNT"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "COUNT(expr?) -> INT"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("COUNT").evaluate(args, row)
    }
}

/// SUM aggregate function lookup
pub struct SumFunction;
impl SqlFunction for SumFunction {
    fn name(&self) -> &str {
        "SUM"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "SUM(numeric) -> NUMERIC"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("SUM").evaluate(args, row)
    }
}

/// AVG aggregate function lookup
pub struct AvgFunction;
impl SqlFunction for AvgFunction {
    fn name(&self) -> &str {
        "AVG"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "AVG(numeric) -> DOUBLE"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("AVG").evaluate(args, row)
    }
}

/// MIN aggregate function lookup
pub struct MinFunction;
impl SqlFunction for MinFunction {
    fn name(&self) -> &str {
        "MIN"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "MIN(expr) -> ANY"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("MIN").evaluate(args, row)
    }
}

/// MAX aggregate function lookup
pub struct MaxFunction;
impl SqlFunction for MaxFunction {
    fn name(&self) -> &str {
        "MAX"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "MAX(expr) -> ANY"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("MAX").evaluate(args, row)
    }
}

/// ARRAY_AGG aggregate function lookup
pub struct ArrayAggFunction;
impl SqlFunction for ArrayAggFunction {
    fn name(&self) -> &str {
        "ARRAY_AGG"
    }
    fn category(&self) -> FunctionCategory {
        FunctionCategory::Aggregate
    }
    fn signature(&self) -> &str {
        "ARRAY_AGG(expr) -> ARRAY"
    }
    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        AggregateLookupFunction::new("ARRAY_AGG").evaluate(args, row)
    }
}
