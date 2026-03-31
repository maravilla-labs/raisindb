//! ElementType file validation (serde-based)

use crate::errors::{codes, FileType, ValidationError, ValidationResult};
use raisin_models::nodes::types::element::element_type::ElementType;

use super::context::ValidationContext;
use super::helpers::format_serde_error;

/// Validate an element type YAML file using serde deserialization
///
/// This is the single source of truth - if serde can parse it into the
/// ElementType struct, it's valid. No duplicate validation logic.
pub fn validate_elementtype(yaml_str: &str, file_path: &str, _ctx: &ValidationContext) -> ValidationResult {
    match serde_yaml::from_str::<ElementType>(yaml_str) {
        Ok(_) => ValidationResult::success(FileType::ElementType),
        Err(e) => {
            let mut result = ValidationResult::success(FileType::ElementType);
            // Format user-friendly error message
            let error_msg = format_serde_error(&e, yaml_str);
            result.add_error(ValidationError::error(
                file_path,
                "",
                codes::YAML_PARSE_ERROR,
                error_msg,
            ));
            result
        }
    }
}
