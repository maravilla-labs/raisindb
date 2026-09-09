//! Numeric functions.
//!
//! The numeric library (ABS, CEIL, ROUND, POWER, ...) lives in the scalar
//! kernels — see `functions::kernel`. This module is the seam kept for numeric
//! functions that need the execution context, and registers nothing today.

use super::registry::FunctionRegistry;

/// Register all numeric functions in the provided registry.
pub fn register_functions(_registry: &mut FunctionRegistry) {}
