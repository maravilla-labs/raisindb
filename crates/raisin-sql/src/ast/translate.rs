//! TRANSLATE statement AST definitions
//!
//! Defines the Abstract Syntax Tree for translation-aware UPDATE statements:
//! - UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'
//! - UPDATE Page FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'
//! - UPDATE Page FOR LOCALE 'de' SET blocks[uuid='550e8400'].text = 'Hallo' WHERE path = '/post'

use serde::{Deserialize, Serialize};

/// A translation path targeting either a node property or a block property
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslationPath {
    /// Simple property path: `title` or `metadata.author`
    /// Converts to JsonPointer: `/title` or `/metadata/author`
    Property(Vec<String>),

    /// Block property path: `blocks[uuid='...'].content.text`
    /// Targets a specific block by UUID, then a property within that block
    BlockProperty {
        /// Array field name (e.g., "blocks", "content")
        array_field: String,
        /// UUID filter value
        block_uuid: String,
        /// Property path within the block
        property_path: Vec<String>,
    },
}

impl TranslationPath {
    /// Create a simple property path
    pub fn property(segments: Vec<String>) -> Self {
        TranslationPath::Property(segments)
    }

    /// Create a block property path
    pub fn block_property(
        array_field: impl Into<String>,
        block_uuid: impl Into<String>,
        property_path: Vec<String>,
    ) -> Self {
        TranslationPath::BlockProperty {
            array_field: array_field.into(),
            block_uuid: block_uuid.into(),
            property_path,
        }
    }

    /// Convert to JsonPointer string for node-level translations
    /// Returns None for block properties (those use separate storage)
    pub fn to_json_pointer(&self) -> Option<String> {
        match self {
            TranslationPath::Property(segments) => Some(format!("/{}", segments.join("/"))),
            TranslationPath::BlockProperty { .. } => None,
        }
    }

    /// Get the block UUID if this is a block property path
    pub fn block_uuid(&self) -> Option<&str> {
        match self {
            TranslationPath::Property(_) => None,
            TranslationPath::BlockProperty { block_uuid, .. } => Some(block_uuid),
        }
    }

    /// Get the property path within a block as JsonPointer
    pub fn block_property_pointer(&self) -> Option<String> {
        match self {
            TranslationPath::Property(_) => None,
            TranslationPath::BlockProperty { property_path, .. } => {
                Some(format!("/{}", property_path.join("/")))
            }
        }
    }

    /// Check if this is a block property path
    pub fn is_block_property(&self) -> bool {
        matches!(self, TranslationPath::BlockProperty { .. })
    }

    /// Check if this is a simple property path
    pub fn is_property(&self) -> bool {
        matches!(self, TranslationPath::Property(_))
    }
}

impl std::fmt::Display for TranslationPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslationPath::Property(segments) => write!(f, "{}", segments.join(".")),
            TranslationPath::BlockProperty {
                array_field,
                block_uuid,
                property_path,
            } => {
                write!(
                    f,
                    "{}[uuid='{}'].{}",
                    array_field,
                    block_uuid,
                    property_path.join(".")
                )
            }
        }
    }
}

/// Value for a translation assignment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TranslationValue {
    /// String value
    String(String),
    /// Integer value
    Integer(i64),
    /// Float value
    Float(f64),
    /// Boolean value
    Boolean(bool),
    /// Null value (to remove a translation)
    Null,
}

impl std::fmt::Display for TranslationValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslationValue::String(s) => write!(f, "'{}'", s),
            TranslationValue::Integer(i) => write!(f, "{}", i),
            TranslationValue::Float(fl) => write!(f, "{}", fl),
            TranslationValue::Boolean(b) => write!(f, "{}", b),
            TranslationValue::Null => write!(f, "NULL"),
        }
    }
}

/// A single translation assignment: `path = value`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationAssignment {
    pub path: TranslationPath,
    pub value: TranslationValue,
}

impl TranslationAssignment {
    /// Create a new translation assignment
    pub fn new(path: TranslationPath, value: TranslationValue) -> Self {
        Self { path, value }
    }
}

impl std::fmt::Display for TranslationAssignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} = {}", self.path, self.value)
    }
}

/// Filter clause for translation statements
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranslateFilter {
    /// Filter by path only: WHERE path = '/post'
    Path(String),
    /// Filter by ID only: WHERE id = 'abc123'
    Id(String),
    /// Filter by path and node_type: WHERE path = '/post' AND node_type = 'Article'
    PathAndType { path: String, node_type: String },
    /// Filter by ID and node_type: WHERE id = 'abc' AND node_type = 'Article'
    IdAndType { id: String, node_type: String },
    /// Filter by node_type only (bulk update): WHERE node_type = 'Article'
    NodeType(String),
}

impl TranslateFilter {
    /// Create a path filter
    pub fn path(path: impl Into<String>) -> Self {
        TranslateFilter::Path(path.into())
    }

    /// Create an ID filter
    pub fn id(id: impl Into<String>) -> Self {
        TranslateFilter::Id(id.into())
    }

    /// Create a path + node_type filter
    pub fn path_and_type(path: impl Into<String>, node_type: impl Into<String>) -> Self {
        TranslateFilter::PathAndType {
            path: path.into(),
            node_type: node_type.into(),
        }
    }

    /// Create an ID + node_type filter
    pub fn id_and_type(id: impl Into<String>, node_type: impl Into<String>) -> Self {
        TranslateFilter::IdAndType {
            id: id.into(),
            node_type: node_type.into(),
        }
    }

    /// Create a node_type only filter
    pub fn node_type(node_type: impl Into<String>) -> Self {
        TranslateFilter::NodeType(node_type.into())
    }
}

impl std::fmt::Display for TranslateFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslateFilter::Path(p) => write!(f, "path = '{}'", p),
            TranslateFilter::Id(i) => write!(f, "id = '{}'", i),
            TranslateFilter::PathAndType { path, node_type } => {
                write!(f, "path = '{}' AND node_type = '{}'", path, node_type)
            }
            TranslateFilter::IdAndType { id, node_type } => {
                write!(f, "id = '{}' AND node_type = '{}'", id, node_type)
            }
            TranslateFilter::NodeType(nt) => write!(f, "node_type = '{}'", nt),
        }
    }
}

/// UPDATE ... FOR LOCALE statement for translating node content
///
/// ```sql
/// UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'
/// UPDATE Page FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'
/// UPDATE Page FOR LOCALE 'de' IN BRANCH 'feature-x' SET blocks[uuid='550e8400'].text = 'Hallo' WHERE path = '/post'
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslateStatement {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// The target locale code (e.g., "de", "fr", "en-US")
    pub locale: String,
    /// Optional branch override (IN BRANCH 'x' clause)
    /// If None, uses the default branch from execution context
    pub branch: Option<String>,
    /// The translation assignments
    pub assignments: Vec<TranslationAssignment>,
    /// The filter clause (required)
    pub filter: Option<TranslateFilter>,
}

impl TranslateStatement {
    /// Create a new TRANSLATE statement
    pub fn new(
        table: impl Into<String>,
        locale: impl Into<String>,
        assignments: Vec<TranslationAssignment>,
        filter: Option<TranslateFilter>,
    ) -> Self {
        Self {
            table: table.into(),
            locale: locale.into(),
            branch: None,
            assignments,
            filter,
        }
    }

    /// Create a new TRANSLATE statement with branch override
    pub fn with_branch(
        table: impl Into<String>,
        locale: impl Into<String>,
        branch: Option<String>,
        assignments: Vec<TranslationAssignment>,
        filter: Option<TranslateFilter>,
    ) -> Self {
        Self {
            table: table.into(),
            locale: locale.into(),
            branch,
            assignments,
            filter,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        "TRANSLATE"
    }

    /// Get only node-level translations (non-block)
    pub fn node_translations(&self) -> impl Iterator<Item = &TranslationAssignment> {
        self.assignments.iter().filter(|a| a.path.is_property())
    }

    /// Get only block-level translations
    pub fn block_translations(&self) -> impl Iterator<Item = &TranslationAssignment> {
        self.assignments
            .iter()
            .filter(|a| a.path.is_block_property())
    }
}

impl std::fmt::Display for TranslateStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UPDATE {} FOR LOCALE '{}'", self.table, self.locale)?;

        // Optional IN BRANCH clause
        if let Some(branch) = &self.branch {
            write!(f, " IN BRANCH '{}'", branch)?;
        }

        write!(f, " SET ")?;

        let assignments: Vec<String> = self.assignments.iter().map(|a| a.to_string()).collect();
        write!(f, "{}", assignments.join(", "))?;

        if let Some(filter) = &self.filter {
            write!(f, " WHERE {}", filter)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translation_path_property() {
        let path = TranslationPath::property(vec!["title".to_string()]);
        assert!(path.is_property());
        assert!(!path.is_block_property());
        assert_eq!(path.to_json_pointer(), Some("/title".to_string()));
        assert_eq!(path.to_string(), "title");
    }

    #[test]
    fn test_translation_path_nested_property() {
        let path = TranslationPath::property(vec!["metadata".to_string(), "author".to_string()]);
        assert!(path.is_property());
        assert_eq!(path.to_json_pointer(), Some("/metadata/author".to_string()));
        assert_eq!(path.to_string(), "metadata.author");
    }

    #[test]
    fn test_translation_path_block_property() {
        let path = TranslationPath::block_property(
            "blocks",
            "550e8400-e29b-41d4",
            vec!["content".to_string(), "text".to_string()],
        );
        assert!(path.is_block_property());
        assert!(!path.is_property());
        assert_eq!(path.to_json_pointer(), None);
        assert_eq!(path.block_uuid(), Some("550e8400-e29b-41d4"));
        assert_eq!(
            path.block_property_pointer(),
            Some("/content/text".to_string())
        );
        assert_eq!(
            path.to_string(),
            "blocks[uuid='550e8400-e29b-41d4'].content.text"
        );
    }

    #[test]
    fn test_translation_value_display() {
        assert_eq!(
            TranslationValue::String("hello".to_string()).to_string(),
            "'hello'"
        );
        assert_eq!(TranslationValue::Integer(42).to_string(), "42");
        assert_eq!(TranslationValue::Float(3.14).to_string(), "3.14");
        assert_eq!(TranslationValue::Boolean(true).to_string(), "true");
        assert_eq!(TranslationValue::Null.to_string(), "NULL");
    }

    #[test]
    fn test_translate_filter_display() {
        assert_eq!(TranslateFilter::path("/post").to_string(), "path = '/post'");
        assert_eq!(TranslateFilter::id("abc123").to_string(), "id = 'abc123'");
        assert_eq!(
            TranslateFilter::path_and_type("/post", "Article").to_string(),
            "path = '/post' AND node_type = 'Article'"
        );
        assert_eq!(
            TranslateFilter::node_type("BlogPost").to_string(),
            "node_type = 'BlogPost'"
        );
    }

    #[test]
    fn test_translate_statement_display() {
        let stmt = TranslateStatement::new(
            "Page",
            "de",
            vec![TranslationAssignment::new(
                TranslationPath::property(vec!["title".to_string()]),
                TranslationValue::String("Titel".to_string()),
            )],
            Some(TranslateFilter::path("/post")),
        );
        assert_eq!(
            stmt.to_string(),
            "UPDATE Page FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'"
        );
    }

    #[test]
    fn test_translate_statement_multiple_assignments() {
        let stmt = TranslateStatement::new(
            "Article",
            "fr",
            vec![
                TranslationAssignment::new(
                    TranslationPath::property(vec!["title".to_string()]),
                    TranslationValue::String("Titre".to_string()),
                ),
                TranslationAssignment::new(
                    TranslationPath::property(vec!["metadata".to_string(), "author".to_string()]),
                    TranslationValue::String("Jean".to_string()),
                ),
            ],
            Some(TranslateFilter::id("abc123")),
        );
        assert_eq!(
            stmt.to_string(),
            "UPDATE Article FOR LOCALE 'fr' SET title = 'Titre', metadata.author = 'Jean' WHERE id = 'abc123'"
        );
    }

    #[test]
    fn test_translate_statement_block_assignment() {
        let stmt = TranslateStatement::new(
            "Page",
            "de",
            vec![TranslationAssignment::new(
                TranslationPath::block_property(
                    "blocks",
                    "550e8400",
                    vec!["content".to_string(), "text".to_string()],
                ),
                TranslationValue::String("Hallo Welt".to_string()),
            )],
            Some(TranslateFilter::path("/post")),
        );
        assert_eq!(
            stmt.to_string(),
            "UPDATE Page FOR LOCALE 'de' SET blocks[uuid='550e8400'].content.text = 'Hallo Welt' WHERE path = '/post'"
        );
    }

    #[test]
    fn test_node_and_block_translations() {
        let stmt = TranslateStatement::new(
            "Page",
            "de",
            vec![
                TranslationAssignment::new(
                    TranslationPath::property(vec!["title".to_string()]),
                    TranslationValue::String("Titel".to_string()),
                ),
                TranslationAssignment::new(
                    TranslationPath::block_property("blocks", "550e8400", vec!["text".to_string()]),
                    TranslationValue::String("Hallo".to_string()),
                ),
            ],
            Some(TranslateFilter::path("/post")),
        );

        let node_trans: Vec<_> = stmt.node_translations().collect();
        let block_trans: Vec<_> = stmt.block_translations().collect();

        assert_eq!(node_trans.len(), 1);
        assert_eq!(block_trans.len(), 1);
        assert!(node_trans[0].path.is_property());
        assert!(block_trans[0].path.is_block_property());
    }
}
