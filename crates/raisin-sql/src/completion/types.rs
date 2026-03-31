//! Completion types for IDE integration
//!
//! These types are designed to be serializable for WASM export
//! and map directly to Monaco editor completion items.

use serde::{Deserialize, Serialize};

/// A completion suggestion with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    /// Display text shown in the completion list
    pub label: String,
    /// Kind of completion (affects icon in UI)
    pub kind: CompletionKind,
    /// Short detail text (shown next to label)
    pub detail: Option<String>,
    /// Full documentation (shown in docs popup)
    pub documentation: Option<String>,
    /// Text to insert when completion is accepted
    pub insert_text: String,
    /// Whether insert_text contains snippet placeholders
    pub insert_text_format: InsertTextFormat,
    /// Text used for sorting (defaults to label)
    pub sort_text: Option<String>,
    /// Text used for filtering (defaults to label)
    pub filter_text: Option<String>,
}

impl CompletionItem {
    /// Create a simple keyword completion
    pub fn keyword(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            insert_text: label.clone(),
            label,
            kind: CompletionKind::Keyword,
            detail: None,
            documentation: None,
            insert_text_format: InsertTextFormat::PlainText,
            sort_text: None,
            filter_text: None,
        }
    }

    /// Create a table completion
    pub fn table(name: impl Into<String>, is_workspace: bool) -> Self {
        let name = name.into();
        Self {
            insert_text: name.clone(),
            label: name,
            kind: CompletionKind::Table,
            detail: Some(if is_workspace {
                "Workspace".to_string()
            } else {
                "Table".to_string()
            }),
            documentation: None,
            insert_text_format: InsertTextFormat::PlainText,
            sort_text: None,
            filter_text: None,
        }
    }

    /// Create a column completion
    pub fn column(name: impl Into<String>, data_type: impl Into<String>, nullable: bool) -> Self {
        let name = name.into();
        let data_type = data_type.into();
        Self {
            insert_text: name.clone(),
            label: name,
            kind: CompletionKind::Column,
            detail: Some(format!(
                "{}{}",
                data_type,
                if nullable { " (nullable)" } else { "" }
            )),
            documentation: None,
            insert_text_format: InsertTextFormat::PlainText,
            sort_text: None,
            filter_text: None,
        }
    }

    /// Create a function completion
    pub fn function(
        name: impl Into<String>,
        params: &[String],
        return_type: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let params_str = params.join(", ");
        let return_type = return_type.into();
        let category = category.into();

        // Create snippet with placeholders for parameters
        let insert_text = if params.is_empty() {
            format!("{}()", name)
        } else {
            let placeholders: Vec<String> = params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("${{{}:{}}}", i + 1, p))
                .collect();
            format!("{}({})", name, placeholders.join(", "))
        };

        Self {
            label: name.clone(),
            kind: CompletionKind::Function,
            detail: Some(format!("({}) -> {}", params_str, return_type)),
            documentation: Some(format!("**Category:** {}", category)),
            insert_text,
            insert_text_format: InsertTextFormat::Snippet,
            sort_text: None,
            filter_text: None,
        }
    }

    /// Create an aggregate function completion
    pub fn aggregate(
        name: impl Into<String>,
        params: &[String],
        return_type: impl Into<String>,
    ) -> Self {
        let mut item = Self::function(name, params, return_type, "Aggregate");
        item.kind = CompletionKind::Aggregate;
        item
    }

    /// Set detail text
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Set documentation
    pub fn with_documentation(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    /// Set sort priority (lower = higher priority)
    pub fn with_sort_priority(mut self, priority: u8) -> Self {
        self.sort_text = Some(format!("{:02}{}", priority, self.label));
        self
    }
}

/// Kind of completion item (affects icon in UI)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    /// SQL keyword (SELECT, FROM, WHERE, etc.)
    Keyword,
    /// Table name
    Table,
    /// Column name
    Column,
    /// Scalar function
    Function,
    /// Aggregate function (COUNT, SUM, etc.)
    Aggregate,
    /// Code snippet template
    Snippet,
    /// Data type (String, Number, etc.)
    Type,
    /// Table or column alias
    Alias,
    /// Operator (AND, OR, etc.)
    Operator,
}

/// Insert text format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InsertTextFormat {
    /// Plain text, inserted as-is
    PlainText,
    /// Snippet with placeholders ($1, ${2:default}, etc.)
    Snippet,
}

/// Result of completion request
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompletionResult {
    /// Completion items to show
    pub items: Vec<CompletionItem>,
    /// Whether the list is incomplete (should re-fetch on more typing)
    pub is_incomplete: bool,
}

impl CompletionResult {
    /// Create an empty result
    pub fn empty() -> Self {
        Self::default()
    }

    /// Add a completion item
    pub fn add(&mut self, item: CompletionItem) {
        self.items.push(item);
    }

    /// Add multiple completion items
    pub fn extend(&mut self, items: impl IntoIterator<Item = CompletionItem>) {
        self.items.extend(items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_completion() {
        let item = CompletionItem::keyword("SELECT");
        assert_eq!(item.label, "SELECT");
        assert_eq!(item.kind, CompletionKind::Keyword);
        assert_eq!(item.insert_text, "SELECT");
    }

    #[test]
    fn test_table_completion() {
        let item = CompletionItem::table("nodes", false);
        assert_eq!(item.label, "nodes");
        assert_eq!(item.kind, CompletionKind::Table);
        assert_eq!(item.detail, Some("Table".to_string()));
    }

    #[test]
    fn test_column_completion() {
        let item = CompletionItem::column("id", "Text", false);
        assert_eq!(item.label, "id");
        assert_eq!(item.kind, CompletionKind::Column);
        assert_eq!(item.detail, Some("Text".to_string()));
    }

    #[test]
    fn test_function_completion() {
        let item = CompletionItem::function("DEPTH", &["path".to_string()], "Int", "Hierarchy");
        assert_eq!(item.label, "DEPTH");
        assert_eq!(item.kind, CompletionKind::Function);
        assert_eq!(item.insert_text, "DEPTH(${1:path})");
        assert_eq!(item.insert_text_format, InsertTextFormat::Snippet);
    }

    #[test]
    fn test_function_completion_no_params() {
        let item = CompletionItem::function("NOW", &[], "TimestampTz", "Temporal");
        assert_eq!(item.insert_text, "NOW()");
    }
}
