//! Node type-membership built-in function signatures: HAS_MIXIN and IS_A.
//!
//! Both take the node's `properties` object and a type/mixin name and return a
//! boolean. The runtime implementations live in `raisin-sql-execution` and test
//! membership against the node's materialized `$mixins` / `$supertypes` sets.

use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register node type-membership built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    // HAS_MIXIN(properties, mixin_name) -> BOOLEAN
    registry.register(FunctionSignature {
        name: "HAS_MIXIN".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    // IS_A(properties, type_name) -> BOOLEAN
    registry.register(FunctionSignature {
        name: "IS_A".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });
}
