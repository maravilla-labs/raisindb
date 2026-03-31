//! Property and index definitions shared by DDL entity types
//!
//! Contains `PropertyDef`, `PropertyTypeDef`, `IndexTypeDef`, `CompoundIndexDef`,
//! and `DefaultValue` types used across NodeType, Archetype, and ElementType DDLs.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts-export")]
use ts_rs::TS;

/// Property/Field definition used in PROPERTIES/FIELDS clauses
///
/// ```sql
/// title String REQUIRED UNIQUE FULLTEXT DEFAULT 'Untitled' LABEL 'Title' DESCRIPTION 'The title'
/// metadata Object {
///     author String LABEL 'Author Name',
///     date Date
/// } ALLOW_ADDITIONAL_PROPERTIES
/// tags Array OF String
/// ```
///
/// For ALTER statements, supports nested paths:
/// ```sql
/// ALTER NODETYPE 'type' MODIFY PROPERTY 'specs.dimensions.width' Number LABEL 'Width (cm)';
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub struct PropertyDef {
    /// Property name (snake_case identifier) or nested path (e.g., "specs.dimensions.width")
    pub name: String,
    /// Property type
    pub property_type: PropertyTypeDef,
    /// Whether this property is required
    pub required: bool,
    /// Whether values must be unique across nodes
    pub unique: bool,
    /// Index types for this property
    pub index: Vec<IndexTypeDef>,
    /// Default value
    pub default: Option<DefaultValue>,
    /// Whether this property is translatable (i18n)
    pub translatable: bool,
    /// Constraints (min, max, pattern, etc.)
    #[cfg_attr(feature = "ts-export", ts(type = "any"))]
    pub constraints: Option<serde_json::Value>,
    /// Human-readable label (stored in meta.label)
    pub label: Option<String>,
    /// Human-readable description (stored in meta.description)
    pub description: Option<String>,
    /// Display order hint (stored in meta.order)
    pub order: Option<i32>,
    /// For Object types: allow properties not in schema
    pub allow_additional_properties: bool,
}

impl PropertyDef {
    /// Check if this property definition uses a nested path (contains '.')
    pub fn is_nested_path(&self) -> bool {
        self.name.contains('.')
    }

    /// Get the path segments for nested properties
    /// e.g., "specs.dimensions.width" -> ["specs", "dimensions", "width"]
    pub fn path_segments(&self) -> Vec<&str> {
        self.name.split('.').collect()
    }

    /// Get the leaf property name (last segment)
    /// e.g., "specs.dimensions.width" -> "width"
    pub fn leaf_name(&self) -> &str {
        self.name.split('.').next_back().unwrap_or(&self.name)
    }
}

impl Default for PropertyDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            property_type: PropertyTypeDef::String,
            required: false,
            unique: false,
            index: Vec::new(),
            default: None,
            translatable: false,
            constraints: None,
            label: None,
            description: None,
            order: None,
            allow_additional_properties: false,
        }
    }
}

/// Property type definition
///
/// Maps to raisin_models::nodes::properties::schema::PropertyType
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub enum PropertyTypeDef {
    /// Text data
    String,
    /// Numeric values (f64)
    Number,
    /// True/false values
    Boolean,
    /// DateTime with ISO-8601 serialization
    Date,
    /// URL strings
    URL,
    /// Cross-node reference
    Reference,
    /// File/resource with metadata
    Resource,
    /// Nested object with inline field definitions
    Object {
        /// Nested field definitions
        fields: Vec<PropertyDef>,
    },
    /// Ordered collection with item type
    Array {
        /// Type of array items
        items: Box<PropertyTypeDef>,
    },
    /// Composite structure (blocks/elements)
    Composite,
    /// Single element in composite
    Element,
    /// Reference to a NodeType definition
    NodeType,
}

impl std::fmt::Display for PropertyTypeDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyTypeDef::String => write!(f, "String"),
            PropertyTypeDef::Number => write!(f, "Number"),
            PropertyTypeDef::Boolean => write!(f, "Boolean"),
            PropertyTypeDef::Date => write!(f, "Date"),
            PropertyTypeDef::URL => write!(f, "URL"),
            PropertyTypeDef::Reference => write!(f, "Reference"),
            PropertyTypeDef::Resource => write!(f, "Resource"),
            PropertyTypeDef::Object { .. } => write!(f, "Object"),
            PropertyTypeDef::Array { items } => write!(f, "Array OF {}", items),
            PropertyTypeDef::Composite => write!(f, "Composite"),
            PropertyTypeDef::Element => write!(f, "Element"),
            PropertyTypeDef::NodeType => write!(f, "NodeType"),
        }
    }
}

/// Index type definition
///
/// Maps to raisin_models::nodes::properties::schema::IndexType
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub enum IndexTypeDef {
    /// Full-text search (Tantivy)
    Fulltext,
    /// Vector embeddings (HNSW)
    Vector,
    /// Property index (RocksDB exact-match)
    Property,
}

impl std::fmt::Display for IndexTypeDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexTypeDef::Fulltext => write!(f, "FULLTEXT"),
            IndexTypeDef::Vector => write!(f, "VECTOR"),
            IndexTypeDef::Property => write!(f, "PROPERTY_INDEX"),
        }
    }
}

/// Compound index definition for efficient ORDER BY + filter queries
///
/// ```sql
/// COMPOUND_INDEX 'idx_category_status_created' ON (
///   category,
///   status,
///   __created_at DESC
/// )
/// ```
///
/// Columns are ordered: leading equality columns first, optional ordering column last.
/// System fields use __ prefix: __node_type, __created_at, __updated_at
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub struct CompoundIndexDef {
    /// Unique name for the index (e.g., 'idx_category_status_created')
    pub name: String,
    /// Columns in order: leading equality columns, then optional ordering column
    pub columns: Vec<CompoundIndexColumnDef>,
    /// If true, last column is used for ORDER BY (ASC/DESC matters)
    pub has_order_column: bool,
}

/// Column definition within a compound index
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub struct CompoundIndexColumnDef {
    /// Property name or system field (__node_type, __created_at, __updated_at)
    pub property: String,
    /// Sort direction (only applies when this is the ordering column)
    /// true = ascending, false = descending
    pub ascending: bool,
}

impl Default for CompoundIndexColumnDef {
    fn default() -> Self {
        Self {
            property: String::new(),
            ascending: true, // default to ascending
        }
    }
}

/// Default value for a property
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(TS))]
#[cfg_attr(feature = "ts-export", ts(export, export_to = "ddl/"))]
pub enum DefaultValue {
    /// String literal
    String(String),
    /// Numeric literal
    Number(f64),
    /// Boolean literal
    Boolean(bool),
    /// Null value
    Null,
}

impl std::fmt::Display for DefaultValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefaultValue::String(s) => write!(f, "'{}'", s),
            DefaultValue::Number(n) => write!(f, "{}", n),
            DefaultValue::Boolean(b) => write!(f, "{}", b),
            DefaultValue::Null => write!(f, "NULL"),
        }
    }
}
