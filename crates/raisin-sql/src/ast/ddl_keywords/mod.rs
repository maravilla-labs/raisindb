//! DDL Keywords for Monaco editor integration
//!
//! This module provides keyword definitions and documentation
//! that can be exported to TypeScript for IDE support.

mod branch_keywords;
mod ddl_statements;
mod functions;
mod property_types;
mod types;

// Re-export public types
pub use types::{DdlKeywords, KeywordCategory, KeywordInfo};

impl DdlKeywords {
    /// Generate the complete keyword list with documentation
    pub fn all() -> Self {
        let mut keywords = Vec::new();

        // DDL statements, schema objects, and clauses
        keywords.extend(ddl_statements::statement_keywords());
        keywords.extend(ddl_statements::schema_object_keywords());
        keywords.extend(ddl_statements::clause_keywords());

        // Property types, modifiers, flags, and operators
        keywords.extend(property_types::property_type_keywords());
        keywords.extend(property_types::modifier_keywords());
        keywords.extend(property_types::flag_keywords());
        keywords.extend(property_types::operator_keywords());

        // Functions (hierarchy, fulltext, vector, JSON, aggregate, window)
        keywords.extend(functions::hierarchy_function_keywords());
        keywords.extend(functions::fulltext_function_keywords());
        keywords.extend(functions::vector_function_keywords());
        keywords.extend(functions::json_function_keywords());
        keywords.extend(functions::aggregate_function_keywords());
        keywords.extend(functions::window_function_keywords());

        // Branch management keywords
        keywords.extend(branch_keywords::branch_keywords());

        Self { keywords }
    }
}

/// Export keywords as JSON for TypeScript consumption
#[cfg(feature = "ts-export")]
pub fn export_keywords_json() -> String {
    serde_json::to_string_pretty(&DdlKeywords::all()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords_count() {
        let keywords = DdlKeywords::all();
        // Ensure we have a reasonable number of keywords
        assert!(
            keywords.keywords.len() > 50,
            "Expected more than 50 keywords"
        );
    }

    #[test]
    fn test_all_keywords_have_description() {
        let keywords = DdlKeywords::all();
        for kw in &keywords.keywords {
            assert!(
                !kw.description.is_empty(),
                "Keyword {} has no description",
                kw.keyword
            );
        }
    }
}
