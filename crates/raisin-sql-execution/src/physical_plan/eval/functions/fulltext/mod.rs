//! Full-text search functions
//!
//! This module contains functions for full-text search operations:
//! - TS_RANK: Get full-text search ranking score

mod ts_rank;

pub use ts_rank::TsRankFunction;

use super::registry::FunctionRegistry;

/// Register all full-text search functions in the provided registry
///
/// This function is called during registry initialization to register
/// all full-text search functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(TsRankFunction));
}
