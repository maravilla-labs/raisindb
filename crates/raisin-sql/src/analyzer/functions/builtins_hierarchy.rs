use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register hierarchy and graph built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    registry.register(FunctionSignature {
        name: "PATH_STARTS_WITH".into(),
        params: vec![DataType::Path, DataType::Path],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    registry.register(FunctionSignature {
        name: "PARENT".into(),
        params: vec![DataType::Path],
        return_type: DataType::Nullable(Box::new(DataType::Path)),
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    registry.register(FunctionSignature {
        name: "PARENT".into(),
        params: vec![DataType::Path, DataType::Int],
        return_type: DataType::Nullable(Box::new(DataType::Path)),
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    registry.register(FunctionSignature {
        name: "DEPTH".into(),
        params: vec![DataType::Path],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    registry.register(FunctionSignature {
        name: "ANCESTOR".into(),
        params: vec![DataType::Path, DataType::Int],
        return_type: DataType::Nullable(Box::new(DataType::Path)),
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    registry.register(FunctionSignature {
        name: "CHILD_OF".into(),
        params: vec![DataType::Path],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    // DESCENDANT_OF(parent_path) - all descendants (unlimited depth)
    registry.register(FunctionSignature {
        name: "DESCENDANT_OF".into(),
        params: vec![DataType::Path],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    // DESCENDANT_OF(parent_path, max_depth) - descendants up to max_depth levels
    registry.register(FunctionSignature {
        name: "DESCENDANT_OF".into(),
        params: vec![DataType::Path, DataType::Int],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    // REFERENCES(target) - check if current node references target
    // Uses reverse reference index for efficient lookup
    registry.register(FunctionSignature {
        name: "REFERENCES".into(),
        params: vec![DataType::Text], // 'workspace:/path' format
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Hierarchy,
    });

    // NEIGHBORS(node_id, direction, relation_type) - Returns array of neighbor node IDs
    // Direction: 'OUT' (outgoing), 'IN' (incoming), 'BOTH'
    // relation_type: Optional filter by relation type
    registry.register(FunctionSignature {
        name: "NEIGHBORS".into(),
        params: vec![DataType::Text, DataType::Text, DataType::Text],
        return_type: DataType::Array(Box::new(DataType::Text)),
        is_deterministic: false, // Result depends on current revision
        category: FunctionCategory::Hierarchy, // TODO: Add Graph category
    });

    // RESOLVE(jsonb) - Resolve all references in a JSONB value (depth=1)
    registry.register(FunctionSignature {
        name: "RESOLVE".into(),
        params: vec![DataType::JsonB],
        return_type: DataType::JsonB,
        is_deterministic: false, // Result depends on current data state
        category: FunctionCategory::Hierarchy,
    });

    // RESOLVE(jsonb, depth) - Resolve references with depth control
    registry.register(FunctionSignature {
        name: "RESOLVE".into(),
        params: vec![DataType::JsonB, DataType::Int],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Hierarchy,
    });
}
