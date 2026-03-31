//! WASM bindings for Raisin Expression Language (REL)
//!
//! Provides real-time REL validation and evaluation in the browser
//! by exposing the Rust parser via WebAssembly.
//!
//! Submodules:
//! - `types` - Type definitions (ValidationError, ValidationResult, etc.)
//! - `completions` - Completion suggestions for REL expressions

mod completions;
mod types;

use raisin_rel::{evaluate, parse, EvalContext, Expr};
use types::{
    EvaluationResult, ParseResultJs, StringifyResult, ValidationError, ValidationResult,
};
use wasm_bindgen::prelude::*;

// =============================================================================
// WASM Exports
// =============================================================================

/// Initialize the WASM module (sets up panic hook for better error messages)
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Validate a REL expression string
///
/// Returns a ValidationResult with:
/// - valid: true if expression is syntactically correct
/// - errors: array of ValidationError if invalid
///
/// @param expression - The REL expression string to validate
/// @returns ValidationResult as JsValue
#[wasm_bindgen]
pub fn validate_expression(expression: &str) -> JsValue {
    let result = match parse(expression) {
        Ok(_) => ValidationResult {
            valid: true,
            errors: vec![],
        },
        Err(e) => ValidationResult {
            valid: false,
            errors: vec![ValidationError {
                line: e.line(),
                column: e.column(),
                end_line: e.line(),
                end_column: e.column() + 10,
                message: e.to_string(),
                severity: "error".to_string(),
            }],
        },
    };

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Evaluate a REL expression against a context
///
/// @param expression - The REL expression string
/// @param context - JSON object with variables (e.g., { input: { value: 10 } })
/// @returns EvaluationResult as JsValue
#[wasm_bindgen]
pub fn evaluate_expression(expression: &str, context: JsValue) -> JsValue {
    // Parse expression
    let expr = match parse(expression) {
        Ok(e) => e,
        Err(e) => {
            let result = EvaluationResult {
                success: false,
                value: None,
                error: Some(format!("Parse error: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    // Build context from JS object
    let ctx = match build_context(context) {
        Ok(c) => c,
        Err(e) => {
            let result = EvaluationResult {
                success: false,
                value: None,
                error: Some(e),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    // Evaluate
    let result = match evaluate(&expr, &ctx) {
        Ok(v) => EvaluationResult {
            success: true,
            value: Some(v.to_json()),
            error: None,
        },
        Err(e) => EvaluationResult {
            success: false,
            value: None,
            error: Some(format!("Evaluation error: {}", e)),
        },
    };

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Parse expression and return AST as JSON (for debugging/tooling)
///
/// @param expression - The REL expression string
/// @returns ParseResult as JsValue
#[wasm_bindgen]
pub fn parse_expression(expression: &str) -> JsValue {
    let result = match parse(expression) {
        Ok(ast) => {
            let ast_js = serde_wasm_bindgen::to_value(&ast).ok();
            ParseResultJs {
                success: true,
                ast: ast_js,
                error: JsValue::NULL,
            }
        }
        Err(e) => {
            let error = ValidationError {
                line: e.line(),
                column: e.column(),
                end_line: e.line(),
                end_column: e.column() + 10,
                message: e.to_string(),
                severity: "error".to_string(),
            };
            ParseResultJs {
                success: false,
                ast: None,
                error: serde_wasm_bindgen::to_value(&error).unwrap_or(JsValue::NULL),
            }
        }
    };

    result.into()
}

/// Convert AST JSON back to REL expression string
///
/// Uses the Expr Display implementation to produce canonical REL code.
/// This is the inverse of parse_expression.
///
/// @param ast_json - The AST as a JavaScript object (same format as parse_expression output)
/// @returns StringifyResult as JsValue
#[wasm_bindgen]
pub fn stringify_expression(ast_json: JsValue) -> JsValue {
    let result = match serde_wasm_bindgen::from_value::<Expr>(ast_json) {
        Ok(expr) => StringifyResult {
            success: true,
            code: Some(format!("{}", expr)),
            error: None,
        },
        Err(e) => StringifyResult {
            success: false,
            code: None,
            error: Some(format!("Failed to deserialize AST: {}", e)),
        },
    };

    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Get completion suggestions at cursor position
///
/// @param expression - The partial REL expression
/// @param cursor_offset - The cursor position (byte offset from start)
/// @returns Array of completion suggestions as JsValue
#[wasm_bindgen]
pub fn get_completions(expression: &str, cursor_offset: usize) -> JsValue {
    let items = completions::compute_completions(expression, cursor_offset);
    serde_wasm_bindgen::to_value(&items).unwrap_or(JsValue::NULL)
}

/// Get method suggestions for autocomplete after a dot
/// This is a separate function for better UI integration
#[wasm_bindgen]
pub fn get_method_completions() -> JsValue {
    let methods = completions::get_all_methods();
    serde_wasm_bindgen::to_value(&methods).unwrap_or(JsValue::NULL)
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Build EvalContext from JavaScript value
fn build_context(js_context: JsValue) -> Result<EvalContext, String> {
    if js_context.is_null() || js_context.is_undefined() {
        return Ok(EvalContext::new());
    }

    let json: serde_json::Value = serde_wasm_bindgen::from_value(js_context)
        .map_err(|e| format!("Invalid context format: {}", e))?;

    EvalContext::from_json(json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::completions::compute_completions;
    use super::types::ValidationResult;
    use super::*;

    #[test]
    fn test_validate_valid() {
        let result: ValidationResult = serde_wasm_bindgen::from_value(
            validate_expression("input.value > 10")
        ).unwrap();
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_invalid() {
        let result: ValidationResult = serde_wasm_bindgen::from_value(
            validate_expression("input.value >")
        ).unwrap();
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_completions() {
        let completions = compute_completions("inp", 3);
        assert!(completions.iter().any(|c| c.label == "input"));
    }
}
