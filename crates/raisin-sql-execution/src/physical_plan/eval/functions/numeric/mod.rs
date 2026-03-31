//! Numeric manipulation functions
//!
//! This module contains functions for numeric operations:
//! - ROUND: Round numeric values to specified decimal places

mod round;

pub use round::RoundFunction;

use super::registry::FunctionRegistry;

/// Register all numeric functions in the provided registry
///
/// This function is called during registry initialization to register
/// all numeric manipulation functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(RoundFunction));
}
