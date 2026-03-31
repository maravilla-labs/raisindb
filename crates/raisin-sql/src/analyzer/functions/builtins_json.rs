use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register JSON built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    // JSON extractors
    registry.register(FunctionSignature {
        name: "JSON_VALUE".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSON_QUERY without wrapper clause (Phase 1)
    registry.register(FunctionSignature {
        name: "JSON_QUERY".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSON_QUERY with wrapper clause (Phase 2)
    registry.register(FunctionSignature {
        name: "JSON_QUERY".into(),
        params: vec![DataType::JsonB, DataType::Text, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSON_QUERY with wrapper + ON EMPTY (Phase 3)
    registry.register(FunctionSignature {
        name: "JSON_QUERY".into(),
        params: vec![
            DataType::JsonB,
            DataType::Text,
            DataType::Text,
            DataType::Text,
        ],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSON_QUERY with wrapper + ON EMPTY + ON ERROR (Phase 3)
    registry.register(FunctionSignature {
        name: "JSON_QUERY".into(),
        params: vec![
            DataType::JsonB,
            DataType::Text,
            DataType::Text,
            DataType::Text,
            DataType::Text,
        ],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    registry.register(FunctionSignature {
        name: "JSON_EXISTS".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSON typed extractors
    registry.register(FunctionSignature {
        name: "JSON_GET_TEXT".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    registry.register(FunctionSignature {
        name: "JSON_GET_DOUBLE".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    registry.register(FunctionSignature {
        name: "JSON_GET_INT".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::Int)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    registry.register(FunctionSignature {
        name: "JSON_GET_BOOL".into(),
        params: vec![DataType::JsonB, DataType::Text],
        return_type: DataType::Nullable(Box::new(DataType::Boolean)),
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // TO_JSON / TO_JSONB - Convert any value to JSONB
    registry.register(FunctionSignature {
        name: "TO_JSON".into(),
        params: vec![DataType::Unknown], // Accepts any type
        return_type: DataType::JsonB,
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    registry.register(FunctionSignature {
        name: "TO_JSONB".into(),
        params: vec![DataType::Unknown], // Accepts any type
        return_type: DataType::JsonB,
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSONB_SET(target, path, new_value) - Set value at path in JSONB
    registry.register(FunctionSignature {
        name: "JSONB_SET".into(),
        params: vec![DataType::JsonB, DataType::Text, DataType::Unknown],
        return_type: DataType::JsonB,
        is_deterministic: true,
        category: FunctionCategory::Json,
    });

    // JSONB_SET with optional create_missing parameter (default true)
    registry.register(FunctionSignature {
        name: "JSONB_SET".into(),
        params: vec![
            DataType::JsonB,
            DataType::Text,
            DataType::Unknown,
            DataType::Boolean,
        ],
        return_type: DataType::JsonB,
        is_deterministic: true,
        category: FunctionCategory::Json,
    });
}
