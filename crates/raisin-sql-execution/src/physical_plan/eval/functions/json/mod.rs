//! JSON/JSONB manipulation functions
//!
//! This module contains functions for JSON operations:
//! - JSON_VALUE: Extract scalar value using JSONPath
//! - JSON_QUERY: Extract JSON object/array using JSONPath (SQL:2016)
//! - JSON_EXISTS: Check if JSONPath exists
//! - JSON_GET_TEXT: Simple key extraction as text
//! - JSON_GET_DOUBLE: Extract number as DOUBLE
//! - JSON_GET_INT: Extract integer value
//! - JSON_GET_BOOL: Extract boolean value
//! - TO_JSON/TO_JSONB: Convert values or table rows to JSON
//! - JSONB_SET: Set value at path in JSONB

mod exists;
mod get_bool;
mod get_double;
mod get_int;
mod get_text;
mod query;
mod set;
mod to_json;
mod value;

pub use exists::JsonExistsFunction;
pub use get_bool::JsonGetBoolFunction;
pub use get_double::JsonGetDoubleFunction;
pub use get_int::JsonGetIntFunction;
pub use get_text::JsonGetTextFunction;
pub use query::JsonQueryFunction;
pub use set::JsonbSetFunction;
pub use to_json::{ToJsonFunction, ToJsonbFunction};
pub use value::JsonValueFunction;

use super::registry::FunctionRegistry;

/// Register all JSON functions in the provided registry
///
/// This function is called during registry initialization to register
/// all JSON manipulation functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(JsonValueFunction));
    registry.register(Box::new(JsonQueryFunction));
    registry.register(Box::new(JsonExistsFunction));
    registry.register(Box::new(JsonGetTextFunction));
    registry.register(Box::new(JsonGetDoubleFunction));
    registry.register(Box::new(JsonGetIntFunction));
    registry.register(Box::new(JsonGetBoolFunction));
    registry.register(Box::new(ToJsonFunction));
    registry.register(Box::new(ToJsonbFunction));
    registry.register(Box::new(JsonbSetFunction));
}
