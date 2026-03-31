//! Function registry for RaisinDB SQL analyzer.
//!
//! Provides the function registry, type definitions, and built-in
//! function registrations organized by category.

mod builtins_hierarchy;
mod builtins_json;
mod builtins_scalar;
mod builtins_search;
mod builtins_system;
mod registry;
mod types;

pub use self::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

#[cfg(test)]
mod tests;
