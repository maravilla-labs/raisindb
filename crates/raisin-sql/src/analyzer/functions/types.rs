use std::collections::HashMap;

use crate::analyzer::types::DataType;

/// Function signature describing a built-in function's name, parameters, return type,
/// and metadata.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub name: String,
    pub params: Vec<DataType>,
    pub return_type: DataType,
    pub is_deterministic: bool,
    pub category: FunctionCategory,
}

/// Category of a built-in function, used for classification and documentation.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionCategory {
    Hierarchy,  // PATH_STARTS_WITH, PARENT, DEPTH
    Json,       // JSON_VALUE, JSON_EXISTS, JSON_GET_*
    FullText,   // to_tsvector, to_tsquery (PostgreSQL-style)
    Vector,     // EMBEDDING - vector search functions
    Geospatial, // ST_POINT, ST_DISTANCE, ST_DWITHIN, etc. (PostGIS-compatible)
    Scalar,     // Standard SQL functions
    Aggregate,  // COUNT, SUM, etc.
    Temporal,   // NOW, CURRENT_TIMESTAMP - date/time functions
    System,     // VERSION, CURRENT_SCHEMA, CURRENT_USER, etc.
    Auth,       // RAISIN_AUTH_* - Authentication configuration functions
}

/// Function registry holding all built-in function signatures, indexed by uppercase name.
pub struct FunctionRegistry {
    pub(super) functions: HashMap<String, Vec<FunctionSignature>>,
}
