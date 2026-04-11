//! Error types and validation result structures for schema validation

use serde::{Deserialize, Serialize};

/// Type of fix that can be applied
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FixType {
    /// Can be fixed automatically without user input
    AutoFixable,
    /// Requires user to choose from options or provide input
    NeedsInput,
    /// Must be fixed manually by the user
    Manual,
}

/// Suggested fix for a validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedFix {
    /// Human-readable description of the fix
    pub description: String,
    /// The original value (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_value: Option<String>,
    /// The suggested new value (for AutoFixable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_value: Option<String>,
    /// Available options to choose from (for NeedsInput)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

/// Severity level of a validation issue
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Error - blocks package creation
    Error,
    /// Warning - allows package creation but informs user
    Warning,
}

/// A single validation error or warning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    /// Path to the file being validated
    pub file_path: String,
    /// Line number (1-based, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Column number (1-based, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// JSON path to the field with the error (e.g., "properties[0].name")
    pub field_path: String,
    /// Machine-readable error code (e.g., "INVALID_NODE_TYPE_NAME")
    pub error_code: String,
    /// Human-readable error message
    pub message: String,
    /// Severity level
    pub severity: Severity,
    /// Type of fix available
    pub fix_type: FixType,
    /// Suggested fix (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<SuggestedFix>,
}

impl ValidationError {
    /// Create a new error
    pub fn error(
        file_path: impl Into<String>,
        field_path: impl Into<String>,
        error_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            line: None,
            column: None,
            field_path: field_path.into(),
            error_code: error_code.into(),
            message: message.into(),
            severity: Severity::Error,
            fix_type: FixType::Manual,
            suggested_fix: None,
        }
    }

    /// Create a new warning
    pub fn warning(
        file_path: impl Into<String>,
        field_path: impl Into<String>,
        error_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            line: None,
            column: None,
            field_path: field_path.into(),
            error_code: error_code.into(),
            message: message.into(),
            severity: Severity::Warning,
            fix_type: FixType::Manual,
            suggested_fix: None,
        }
    }

    /// Set line and column
    pub fn with_location(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Set as auto-fixable with suggested new value
    pub fn with_auto_fix(mut self, description: impl Into<String>, new_value: impl Into<String>) -> Self {
        self.fix_type = FixType::AutoFixable;
        self.suggested_fix = Some(SuggestedFix {
            description: description.into(),
            old_value: None,
            new_value: Some(new_value.into()),
            options: None,
        });
        self
    }

    /// Set as auto-fixable with old and new values
    pub fn with_auto_fix_replace(
        mut self,
        description: impl Into<String>,
        old_value: impl Into<String>,
        new_value: impl Into<String>,
    ) -> Self {
        self.fix_type = FixType::AutoFixable;
        self.suggested_fix = Some(SuggestedFix {
            description: description.into(),
            old_value: Some(old_value.into()),
            new_value: Some(new_value.into()),
            options: None,
        });
        self
    }

    /// Set as needing input with options
    pub fn with_options(mut self, description: impl Into<String>, options: Vec<String>) -> Self {
        self.fix_type = FixType::NeedsInput;
        self.suggested_fix = Some(SuggestedFix {
            description: description.into(),
            old_value: None,
            new_value: None,
            options: Some(options),
        });
        self
    }
}

/// Result of validating a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether validation passed (no errors, warnings ok)
    pub success: bool,
    /// Type of file validated
    pub file_type: FileType,
    /// List of errors (block creation)
    pub errors: Vec<ValidationError>,
    /// List of warnings (allow creation)
    pub warnings: Vec<ValidationError>,
}

impl ValidationResult {
    /// Create a successful result
    pub fn success(file_type: FileType) -> Self {
        Self {
            success: true,
            file_type,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Create a result with errors
    pub fn with_errors(file_type: FileType, errors: Vec<ValidationError>) -> Self {
        Self {
            success: errors.is_empty(),
            file_type,
            errors,
            warnings: Vec::new(),
        }
    }

    /// Add an error
    pub fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
        self.success = false;
    }

    /// Add a warning
    pub fn add_warning(&mut self, warning: ValidationError) {
        self.warnings.push(warning);
    }

    /// Merge another result into this one
    pub fn merge(&mut self, other: ValidationResult) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self.success = self.errors.is_empty();
    }
}

/// Type of file being validated
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Manifest,
    NodeType,
    Workspace,
    Content,
    Archetype,
    ElementType,
}

/// Error codes used in validation
pub mod codes {
    // YAML errors
    pub const YAML_SYNTAX_ERROR: &str = "YAML_SYNTAX_ERROR";
    pub const YAML_PARSE_ERROR: &str = "YAML_PARSE_ERROR";

    // Manifest errors
    pub const MISSING_REQUIRED_FIELD: &str = "MISSING_REQUIRED_FIELD";
    pub const INVALID_PACKAGE_NAME: &str = "INVALID_PACKAGE_NAME";
    pub const INVALID_VERSION: &str = "INVALID_VERSION";

    // NodeType errors
    pub const INVALID_NODE_TYPE_NAME: &str = "INVALID_NODE_TYPE_NAME";
    pub const INVALID_PROPERTY_TYPE: &str = "INVALID_PROPERTY_TYPE";
    pub const DUPLICATE_PROPERTY: &str = "DUPLICATE_PROPERTY";
    pub const INVALID_EXTENDS: &str = "INVALID_EXTENDS";
    pub const INVALID_MIXIN: &str = "INVALID_MIXIN";

    // Workspace errors
    pub const INVALID_WORKSPACE_NAME: &str = "INVALID_WORKSPACE_NAME";
    pub const INVALID_ALLOWED_TYPE: &str = "INVALID_ALLOWED_TYPE";

    // Content errors
    pub const INVALID_CONTENT_NODE_TYPE: &str = "INVALID_CONTENT_NODE_TYPE";
    pub const MISSING_REQUIRED_PROPERTY: &str = "MISSING_REQUIRED_PROPERTY";
    pub const INVALID_PROPERTY_VALUE: &str = "INVALID_PROPERTY_VALUE";

    // Reference warnings
    pub const UNKNOWN_NODE_TYPE_REFERENCE: &str = "UNKNOWN_NODE_TYPE_REFERENCE";
    pub const UNKNOWN_WORKSPACE_REFERENCE: &str = "UNKNOWN_WORKSPACE_REFERENCE";
    pub const UNRESOLVABLE_CONTENT_REFERENCE: &str = "UNRESOLVABLE_CONTENT_REFERENCE";
    pub const UNKNOWN_ARCHETYPE_REFERENCE: &str = "UNKNOWN_ARCHETYPE_REFERENCE";
    pub const UNKNOWN_ELEMENT_TYPE_REFERENCE: &str = "UNKNOWN_ELEMENT_TYPE_REFERENCE";

    // Archetype errors
    pub const INVALID_ARCHETYPE_NAME: &str = "INVALID_ARCHETYPE_NAME";
    pub const INVALID_FIELD_TYPE_KEY: &str = "INVALID_FIELD_TYPE_KEY";
    pub const MISSING_FIELD_TYPE: &str = "MISSING_FIELD_TYPE";
    pub const INVALID_FIELD_NAME: &str = "INVALID_FIELD_NAME";

    // ElementType errors
    pub const INVALID_ELEMENT_TYPE_NAME: &str = "INVALID_ELEMENT_TYPE_NAME";
    pub const EMPTY_FIELDS_ARRAY: &str = "EMPTY_FIELDS_ARRAY";

    // Required field validation errors
    pub const MISSING_REQUIRED_ELEMENT_FIELD: &str = "MISSING_REQUIRED_ELEMENT_FIELD";
    pub const MISSING_REQUIRED_ARCHETYPE_FIELD: &str = "MISSING_REQUIRED_ARCHETYPE_FIELD";

    // Composite field UUID validation errors
    pub const COMPOSITE_MISSING_UUID: &str = "COMPOSITE_MISSING_UUID";
    pub const COMPOSITE_DUPLICATE_UUID: &str = "COMPOSITE_DUPLICATE_UUID";
}
