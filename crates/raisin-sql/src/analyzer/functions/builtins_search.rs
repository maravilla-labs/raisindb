use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register full-text search and vector search built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    // Full-text search functions (PostgreSQL-style)
    registry.register(FunctionSignature {
        name: "to_tsvector".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::TSVector,
        is_deterministic: true,
        category: FunctionCategory::FullText,
    });

    registry.register(FunctionSignature {
        name: "to_tsquery".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::TSQuery,
        is_deterministic: true,
        category: FunctionCategory::FullText,
    });

    // FULLTEXT_MATCH(query, language) - Search using Tantivy index
    registry.register(FunctionSignature {
        name: "FULLTEXT_MATCH".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: false, // Depends on index state
        category: FunctionCategory::FullText,
    });

    // Vector search functions
    // EMBEDDING(text) - Generate vector embedding from text
    registry.register(FunctionSignature {
        name: "EMBEDDING".into(),
        params: vec![DataType::Text],
        return_type: DataType::Unknown, // Resolved to Vector(dims) during semantic analysis
        is_deterministic: false,        // External API call
        category: FunctionCategory::Vector,
    });

    // VECTOR_OF(node_ref) / VECTOR_OF(node_ref, chunk_index)
    //
    // A node's OWN STORED vector, read out of `cf::EMBEDDINGS` rather than
    // recomputed. It is only meaningful as argument 1 of `KNN`, which is where
    // it is interpreted (`search::args::parse_query_input`); it is registered
    // here so the analyzer accepts the call at all, exactly as `EMBEDDING` is.
    //
    // Two overloads, resolved by arity: without a chunk index the source must
    // have exactly ONE stored vector, and a chunked document is a loud error
    // naming its chunk count rather than a silent chunk 0.
    registry.register(FunctionSignature {
        name: "VECTOR_OF".into(),
        params: vec![DataType::Text],
        return_type: DataType::Unknown, // A Vector(dims) only the store knows.
        is_deterministic: false,        // Reads the current stored vector.
        category: FunctionCategory::Vector,
    });
    registry.register(FunctionSignature {
        name: "VECTOR_OF".into(),
        params: vec![DataType::Text, DataType::BigInt],
        return_type: DataType::Unknown,
        is_deterministic: false,
        category: FunctionCategory::Vector,
    });

    // VECTOR_L2_DISTANCE(vec1, vec2) - Euclidean distance (L2)
    registry.register(FunctionSignature {
        name: "VECTOR_L2_DISTANCE".into(),
        params: vec![DataType::Unknown, DataType::Unknown],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Vector,
    });

    // VECTOR_COSINE_DISTANCE(vec1, vec2) - Cosine distance
    registry.register(FunctionSignature {
        name: "VECTOR_COSINE_DISTANCE".into(),
        params: vec![DataType::Unknown, DataType::Unknown],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Vector,
    });

    // VECTOR_INNER_PRODUCT(vec1, vec2) - Inner product (negative dot product)
    registry.register(FunctionSignature {
        name: "VECTOR_INNER_PRODUCT".into(),
        params: vec![DataType::Unknown, DataType::Unknown],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Vector,
    });
}
