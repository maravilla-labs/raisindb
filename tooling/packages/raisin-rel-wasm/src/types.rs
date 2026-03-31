//! Type definitions for WASM REL expression bindings

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// A validation error with position information for Monaco editor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Line number (1-based for Monaco)
    pub line: usize,
    /// Column number (1-based for Monaco)
    pub column: usize,
    /// End line number
    pub end_line: usize,
    /// End column number
    pub end_column: usize,
    /// Error message
    pub message: String,
    /// Severity: "error" or "warning"
    pub severity: String,
}

/// Result of REL validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether the expression is valid
    pub valid: bool,
    /// List of errors found (empty if valid)
    pub errors: Vec<ValidationError>,
}

/// Result of REL evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Whether evaluation succeeded
    pub success: bool,
    /// The result value (if successful)
    pub value: Option<serde_json::Value>,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// AST parse result for tooling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    /// Whether parsing succeeded
    pub success: bool,
    /// The AST as JSON (if successful)
    pub ast: Option<serde_json::Value>,
    /// Error information (if failed)
    pub error: Option<ValidationError>,
}

/// Result of AST stringification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringifyResult {
    /// Whether stringification succeeded
    pub success: bool,
    /// The REL code (if successful)
    pub code: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// Completion suggestion item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    /// Display label
    pub label: String,
    /// Kind of completion (keyword, function, variable, operator)
    pub kind: String,
    /// Text to insert
    pub insert_text: String,
    /// Documentation/description
    pub detail: Option<String>,
}

/// Parse result for JavaScript (with JsValue AST)
pub(crate) struct ParseResultJs {
    pub success: bool,
    pub ast: Option<JsValue>,
    pub error: JsValue,
}

impl From<ParseResultJs> for JsValue {
    fn from(result: ParseResultJs) -> Self {
        let obj = js_sys::Object::new();
        js_sys::Reflect::set(&obj, &"success".into(), &result.success.into()).unwrap();
        if let Some(ast) = result.ast {
            js_sys::Reflect::set(&obj, &"ast".into(), &ast).unwrap();
        } else {
            js_sys::Reflect::set(&obj, &"ast".into(), &JsValue::NULL).unwrap();
        }
        js_sys::Reflect::set(&obj, &"error".into(), &result.error).unwrap();
        obj.into()
    }
}
