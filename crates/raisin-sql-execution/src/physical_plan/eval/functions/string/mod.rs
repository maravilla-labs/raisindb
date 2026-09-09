//! String manipulation functions
//!
//! This module contains functions for string operations:
//! UPPER, LOWER and the rest of the pure string library are scalar kernels
//! (see `functions::kernel`). What is left here needs behaviour a strict,
//! pure kernel cannot have:
//! - COALESCE: Return first non-NULL value
//! - NULLIF: Return NULL if two values are equal

mod coalesce;
mod nullif;

pub use coalesce::CoalesceFunction;
pub use nullif::NullIfFunction;

use super::registry::FunctionRegistry;

/// Register all string functions in the provided registry
///
/// This function is called during registry initialization to register
/// all string manipulation functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(CoalesceFunction));
    registry.register(Box::new(NullIfFunction));
}
