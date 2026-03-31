//! Function registry for SQL function lookup
//!
//! This module provides a centralized registry for all SQL functions.
//! Functions are registered by name (case-insensitive) and can be looked up
//! dynamically during query evaluation.
//!
//! # Design
//! - Case-insensitive function name lookup (all names stored in uppercase)
//! - Thread-safe access via `&self` methods
//! - Functions stored as trait objects (`Box<dyn SqlFunction>`)
//! - O(1) lookup performance via HashMap

use super::traits::SqlFunction;
use std::collections::HashMap;

/// Registry for SQL functions with case-insensitive lookup
///
/// The registry stores function implementations as trait objects and provides
/// efficient lookup by function name. All function names are normalized to
/// uppercase for case-insensitive matching.
///
/// # Example
/// ```rust,ignore
/// let mut registry = FunctionRegistry::new();
/// registry.register(Box::new(UpperFunction));
/// registry.register(Box::new(LowerFunction));
///
/// // Case-insensitive lookup
/// assert!(registry.get("upper").is_some());
/// assert!(registry.get("UPPER").is_some());
/// assert!(registry.get("Upper").is_some());
/// ```
pub struct FunctionRegistry {
    /// Map from uppercase function name to function implementation
    functions: HashMap<String, Box<dyn SqlFunction>>,
}

impl FunctionRegistry {
    /// Create a new empty function registry
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }

    /// Register a function in the registry
    ///
    /// The function name is normalized to uppercase for case-insensitive lookup.
    /// If a function with the same name already exists, it will be replaced.
    ///
    /// # Arguments
    /// * `function` - Boxed function implementation
    ///
    /// # Example
    /// ```rust,ignore
    /// registry.register(Box::new(UpperFunction));
    /// ```
    pub fn register(&mut self, function: Box<dyn SqlFunction>) {
        let name = function.name().to_uppercase();
        self.functions.insert(name, function);
    }

    /// Look up a function by name (case-insensitive)
    ///
    /// # Arguments
    /// * `name` - Function name (case-insensitive)
    ///
    /// # Returns
    /// * `Some(&dyn SqlFunction)` - Reference to the function if found
    /// * `None` - If no function with this name is registered
    ///
    /// # Example
    /// ```rust,ignore
    /// if let Some(func) = registry.get("upper") {
    ///     let result = func.evaluate(&args, &row)?;
    /// }
    /// ```
    pub fn get(&self, name: &str) -> Option<&dyn SqlFunction> {
        let normalized = name.to_uppercase();
        self.functions.get(&normalized).map(|b| b.as_ref())
    }

    /// Get the number of registered functions
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    /// Get all registered function names (in uppercase)
    ///
    /// Useful for debugging and introspection.
    pub fn function_names(&self) -> Vec<&str> {
        self.functions.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
