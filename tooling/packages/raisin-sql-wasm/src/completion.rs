//! SQL completion and function signature APIs

use raisin_sql::analyzer::functions::FunctionRegistry;
use raisin_sql::analyzer::StaticCatalog;
use raisin_sql::completion::CompletionProvider;
use wasm_bindgen::prelude::*;

use crate::types::{FunctionSignatureInfo, TABLE_CATALOG};

/// Get context-aware completion suggestions at cursor position
///
/// @param sql - The SQL text being edited
/// @param cursor_offset - The cursor position (byte offset from start)
/// @returns CompletionResult with items array and is_incomplete flag
#[wasm_bindgen]
pub fn get_completions(sql: &str, cursor_offset: usize) -> JsValue {
    // Build catalog from TABLE_CATALOG with registered workspaces
    let catalog = TABLE_CATALOG.with(|c| {
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace_name in c.borrow().keys() {
            catalog.register_workspace(workspace_name.clone());
        }
        catalog
    });

    let functions = FunctionRegistry::default();
    let provider = CompletionProvider::new(&catalog, &functions);
    let result = provider.provide_completions(sql, cursor_offset);

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Get function signatures for a specific function name
///
/// @param function_name - The function name (case-insensitive)
/// @returns Array of FunctionSignatureInfo or null if not found
#[wasm_bindgen]
pub fn get_function_signatures(function_name: &str) -> JsValue {
    let registry = FunctionRegistry::default();

    if let Some(signatures) = registry.get_signatures(function_name) {
        let sig_infos: Vec<FunctionSignatureInfo> = signatures
            .iter()
            .map(|sig| FunctionSignatureInfo {
                name: sig.name.clone(),
                params: sig.params.iter().map(format_data_type).collect(),
                return_type: format_data_type(&sig.return_type),
                category: format!("{:?}", sig.category),
                is_deterministic: sig.is_deterministic,
            })
            .collect();

        serde_wasm_bindgen::to_value(&sig_infos).unwrap_or(JsValue::NULL)
    } else {
        JsValue::NULL
    }
}

/// Get all available functions with their signatures
///
/// @returns Array of FunctionSignatureInfo for all functions
#[wasm_bindgen]
pub fn get_all_functions() -> JsValue {
    let registry = FunctionRegistry::default();

    let function_names = [
        "PATH_STARTS_WITH",
        "PARENT",
        "DEPTH",
        "ANCESTOR",
        "CHILD_OF",
        "DESCENDANT_OF",
        "JSON_VALUE",
        "JSON_QUERY",
        "JSON_EXISTS",
        "JSON_GET_TEXT",
        "JSON_GET_DOUBLE",
        "JSON_GET_INT",
        "JSON_GET_BOOL",
        "to_tsvector",
        "to_tsquery",
        "FULLTEXT_MATCH",
        "EMBEDDING",
        "VECTOR_L2_DISTANCE",
        "VECTOR_COSINE_DISTANCE",
        "VECTOR_INNER_PRODUCT",
        "NEIGHBORS",
        "LOWER",
        "UPPER",
        "LENGTH",
        "ROUND",
        "COUNT",
        "SUM",
        "AVG",
        "MIN",
        "MAX",
        "ARRAY_AGG",
        "COALESCE",
        "NOW",
    ];

    let mut all_sigs: Vec<FunctionSignatureInfo> = Vec::new();

    for name in function_names {
        if let Some(signatures) = registry.get_signatures(name) {
            for sig in signatures {
                all_sigs.push(FunctionSignatureInfo {
                    name: sig.name.clone(),
                    params: sig.params.iter().map(format_data_type).collect(),
                    return_type: format_data_type(&sig.return_type),
                    category: format!("{:?}", sig.category),
                    is_deterministic: sig.is_deterministic,
                });
            }
        }
    }

    serde_wasm_bindgen::to_value(&all_sigs).unwrap_or(JsValue::NULL)
}

/// Format DataType for display
fn format_data_type(dt: &raisin_sql::DataType) -> String {
    use raisin_sql::DataType;
    match dt {
        DataType::Int => "Int".to_string(),
        DataType::BigInt => "BigInt".to_string(),
        DataType::Double => "Double".to_string(),
        DataType::Boolean => "Boolean".to_string(),
        DataType::Text => "Text".to_string(),
        DataType::Uuid => "Uuid".to_string(),
        DataType::TimestampTz => "Timestamp".to_string(),
        DataType::Interval => "Interval".to_string(),
        DataType::Path => "Path".to_string(),
        DataType::JsonB => "JsonB".to_string(),
        DataType::Vector(n) => format!("Vector({})", n),
        DataType::Array(inner) => format!("Array<{}>", format_data_type(inner)),
        DataType::Nullable(inner) => format!("{}?", format_data_type(inner)),
        DataType::TSVector => "TSVector".to_string(),
        DataType::TSQuery => "TSQuery".to_string(),
        DataType::Geometry => "Geometry".to_string(),
        DataType::Unknown => "Any".to_string(),
    }
}
