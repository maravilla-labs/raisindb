//! Type definitions for WASM SQL validation and completion

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

/// A validation error with position information for Monaco editor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ValidationError {
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

/// Result of SQL validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ValidationResult {
    /// Whether the SQL is valid
    pub success: bool,
    /// List of errors found
    pub errors: Vec<ValidationError>,
}

/// Column definition for table catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ColumnDef {
    /// Column name
    pub name: String,
    /// Data type (e.g., "String", "Number", "Boolean")
    pub data_type: String,
    /// Whether the column is nullable
    pub nullable: bool,
}

/// Table definition for catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TableDef {
    /// Table name
    pub name: String,
    /// Column definitions
    pub columns: Vec<ColumnDef>,
}

/// Function signature information for signature help
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FunctionSignatureInfo {
    /// Function name
    pub name: String,
    /// Parameter types (e.g., ["Path", "Int"])
    pub params: Vec<String>,
    /// Return type (e.g., "Int")
    pub return_type: String,
    /// Category (e.g., "Hierarchy", "Json", "Aggregate")
    pub category: String,
    /// Whether the function is deterministic
    pub is_deterministic: bool,
}

// Thread-local table catalog storage
thread_local! {
    pub(crate) static TABLE_CATALOG: RefCell<HashMap<String, TableDef>> = RefCell::new(HashMap::new());
}
