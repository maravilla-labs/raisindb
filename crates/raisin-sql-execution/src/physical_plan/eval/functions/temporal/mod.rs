//! Temporal/datetime functions
//!
//! This module contains functions for date and time operations:
//! - NOW: Return current UTC timestamp
//! - CURRENT_TIMESTAMP: Alias for NOW (future)
//! - DATE arithmetic operations (future)

mod now;

pub use now::NowFunction;

use super::registry::FunctionRegistry;

/// Register all temporal functions in the provided registry
///
/// This function is called during registry initialization to register
/// all date/time functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(NowFunction));
}
