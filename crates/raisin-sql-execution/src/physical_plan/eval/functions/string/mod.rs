//! String manipulation functions
//!
//! This module contains functions for string operations:
//! - UPPER: Convert text to uppercase
//! - LOWER: Convert text to lowercase
//! - COALESCE: Return first non-NULL value
//! - NULLIF: Return NULL if two values are equal

mod coalesce;
mod lower;
mod nullif;
mod upper;

pub use coalesce::CoalesceFunction;
pub use lower::LowerFunction;
pub use nullif::NullIfFunction;
pub use upper::UpperFunction;

use super::registry::FunctionRegistry;

/// Register all string functions in the provided registry
///
/// This function is called during registry initialization to register
/// all string manipulation functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(UpperFunction));
    registry.register(Box::new(LowerFunction));
    registry.register(Box::new(CoalesceFunction));
    registry.register(Box::new(NullIfFunction));
}
