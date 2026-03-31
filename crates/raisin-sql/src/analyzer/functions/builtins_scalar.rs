use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register scalar, aggregate, and temporal built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    // Scalar string functions
    registry.register(FunctionSignature {
        name: "LOWER".into(),
        params: vec![DataType::Text],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    registry.register(FunctionSignature {
        name: "UPPER".into(),
        params: vec![DataType::Text],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    registry.register(FunctionSignature {
        name: "LENGTH".into(),
        params: vec![DataType::Text],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    // Math functions
    registry.register(FunctionSignature {
        name: "ROUND".into(),
        params: vec![DataType::Double, DataType::Int],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    registry.register(FunctionSignature {
        name: "ROUND".into(),
        params: vec![DataType::Double],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    // COALESCE - variadic function returning first non-NULL value
    registry.register(FunctionSignature {
        name: "COALESCE".into(),
        params: vec![DataType::Unknown],
        return_type: DataType::Unknown,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    // NULLIF - return NULL if two values are equal
    registry.register(FunctionSignature {
        name: "NULLIF".into(),
        params: vec![DataType::Unknown, DataType::Unknown],
        return_type: DataType::Unknown,
        is_deterministic: true,
        category: FunctionCategory::Scalar,
    });

    // Aggregate functions
    registry.register(FunctionSignature {
        name: "COUNT".into(),
        params: vec![],
        return_type: DataType::BigInt,
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    // COUNT(*) and COUNT(column)
    registry.register(FunctionSignature {
        name: "COUNT".into(),
        params: vec![DataType::Unknown],
        return_type: DataType::BigInt,
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    registry.register(FunctionSignature {
        name: "SUM".into(),
        params: vec![DataType::Double],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    registry.register(FunctionSignature {
        name: "AVG".into(),
        params: vec![DataType::Double],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    registry.register(FunctionSignature {
        name: "MIN".into(),
        params: vec![DataType::Unknown],
        return_type: DataType::Unknown,
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    registry.register(FunctionSignature {
        name: "MAX".into(),
        params: vec![DataType::Unknown],
        return_type: DataType::Unknown,
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    registry.register(FunctionSignature {
        name: "ARRAY_AGG".into(),
        params: vec![DataType::Unknown],
        return_type: DataType::Unknown,
        is_deterministic: true,
        category: FunctionCategory::Aggregate,
    });

    // Temporal functions
    // NOW() - Return current UTC timestamp
    registry.register(FunctionSignature {
        name: "NOW".into(),
        params: vec![],
        return_type: DataType::TimestampTz,
        is_deterministic: false,
        category: FunctionCategory::Temporal,
    });
}
