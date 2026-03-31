use std::collections::HashMap;

use crate::analyzer::types::DataType;

use super::types::{FunctionRegistry, FunctionSignature};

impl Default for FunctionRegistry {
    fn default() -> Self {
        let mut registry = FunctionRegistry {
            functions: HashMap::new(),
        };

        super::builtins_hierarchy::register(&mut registry);
        super::builtins_json::register(&mut registry);
        super::builtins_search::register(&mut registry);
        super::builtins_scalar::register(&mut registry);
        super::builtins_system::register(&mut registry);

        registry
    }
}

impl FunctionRegistry {
    /// Register a single function signature into the registry.
    pub(super) fn register(&mut self, sig: FunctionSignature) {
        self.functions
            .entry(sig.name.to_uppercase())
            .or_default()
            .push(sig);
    }

    /// Resolve function by name and argument types.
    /// Returns the best matching signature considering type coercion.
    pub fn resolve(&self, name: &str, arg_types: &[DataType]) -> Option<&FunctionSignature> {
        let name_upper = name.to_uppercase();
        let signatures = self.functions.get(&name_upper)?;

        // First, try exact match
        for sig in signatures {
            if sig.params.len() == arg_types.len()
                && sig
                    .params
                    .iter()
                    .zip(arg_types.iter())
                    .all(|(param, arg)| param == arg)
            {
                return Some(sig);
            }
        }

        // Then try with coercion
        signatures
            .iter()
            .find(|&sig| {
                sig.params.len() == arg_types.len()
                    && sig
                        .params
                        .iter()
                        .zip(arg_types.iter())
                        .all(|(param, arg)| arg.can_coerce_to(param))
            })
            .map(|v| v as _)
    }

    /// Get all signatures for a function name.
    pub fn get_signatures(&self, name: &str) -> Option<&[FunctionSignature]> {
        self.functions
            .get(&name.to_uppercase())
            .map(|v| v.as_slice())
    }
}
