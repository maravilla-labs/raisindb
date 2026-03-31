//! Aggregate function lookup
//!
//! This module handles aggregate functions (COUNT, SUM, AVG, MIN, MAX, ARRAY_AGG).
//! Aggregate functions are pre-computed by the HashAggregate operator, and these
//! implementations simply look up the pre-computed values from the row.
//!
//! The naming module provides utilities for generating canonical column names
//! that are used to match aggregate results with their expressions.

mod lookup;
pub mod naming;

pub use lookup::{
    ArrayAggFunction, AvgFunction, CountFunction, MaxFunction, MinFunction, SumFunction,
};

use super::registry::FunctionRegistry;

/// Register all aggregate functions in the provided registry
///
/// This function is called during registry initialization to register
/// all aggregate lookup functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(CountFunction));
    registry.register(Box::new(SumFunction));
    registry.register(Box::new(AvgFunction));
    registry.register(Box::new(MinFunction));
    registry.register(Box::new(MaxFunction));
    registry.register(Box::new(ArrayAggFunction));
}
