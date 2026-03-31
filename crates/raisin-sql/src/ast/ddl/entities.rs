//! DDL entity types: NodeType, Archetype, and ElementType
//!
//! Defines CREATE, ALTER, and DROP AST nodes for the three main schema
//! entity types in RaisinDB.

use serde::{Deserialize, Serialize};

use super::properties::{CompoundIndexDef, PropertyDef};

// =============================================================================
// NodeType DDL
// =============================================================================

/// CREATE NODETYPE statement
///
/// ```sql
/// CREATE NODETYPE 'myapp:Article'
///   EXTENDS 'raisin:Page'
///   MIXINS ('myapp:Publishable', 'myapp:SEO')
///   DESCRIPTION 'Blog article content type'
///   ICON 'article'
///   PROPERTIES (
///     title String REQUIRED FULLTEXT,
///     slug String REQUIRED UNIQUE
///   )
///   ALLOWED_CHILDREN ('myapp:Paragraph', 'myapp:Image')
///   COMPOUND_INDEX 'idx_category_status_created' ON (
///     category,
///     status,
///     __created_at DESC
///   )
///   PUBLISHABLE
///   VERSIONABLE;
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateNodeType {
    /// Type name with namespace, e.g., 'myapp:Article'
    pub name: String,
    /// Parent type to extend
    pub extends: Option<String>,
    /// Mixin types to include
    pub mixins: Vec<String>,
    /// Human-readable description
    pub description: Option<String>,
    /// Icon identifier
    pub icon: Option<String>,
    /// Property definitions
    pub properties: Vec<PropertyDef>,
    /// Allowed child node types
    pub allowed_children: Vec<String>,
    /// Required child node types
    pub required_nodes: Vec<String>,
    /// Initial structure template (optional, complex)
    pub initial_structure: Option<serde_json::Value>,
    /// Compound indexes for efficient ORDER BY + filter queries
    pub compound_indexes: Vec<CompoundIndexDef>,
    /// Whether nodes of this type can be versioned
    pub versionable: bool,
    /// Whether nodes of this type can be published
    pub publishable: bool,
    /// Whether changes are audited
    pub auditable: bool,
    /// Whether nodes are indexed
    pub indexable: bool,
    /// Strict mode (reject unknown properties)
    pub strict: bool,
}

impl Default for CreateNodeType {
    fn default() -> Self {
        Self {
            name: String::new(),
            extends: None,
            mixins: Vec::new(),
            description: None,
            icon: None,
            properties: Vec::new(),
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: Vec::new(),
            versionable: false,
            publishable: false,
            auditable: false,
            indexable: true, // default to indexable
            strict: false,
        }
    }
}

/// ALTER NODETYPE statement
///
/// ```sql
/// ALTER NODETYPE 'myapp:Article'
///   ADD PROPERTY subtitle String FULLTEXT
///   DROP PROPERTY legacy_field
///   SET DESCRIPTION = 'Updated description';
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlterNodeType {
    /// Type name to alter
    pub name: String,
    /// Alterations to apply
    pub alterations: Vec<NodeTypeAlteration>,
}

/// Individual alteration operation for NodeType
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeTypeAlteration {
    /// Add a new property
    AddProperty(PropertyDef),
    /// Drop an existing property
    DropProperty(String),
    /// Modify an existing property
    ModifyProperty(PropertyDef),
    /// Set description
    SetDescription(String),
    /// Set icon
    SetIcon(String),
    /// Set extends (parent type)
    SetExtends(Option<String>),
    /// Set allowed children
    SetAllowedChildren(Vec<String>),
    /// Set required nodes
    SetRequiredNodes(Vec<String>),
    /// Add a mixin
    AddMixin(String),
    /// Drop a mixin
    DropMixin(String),
    /// Set versionable flag
    SetVersionable(bool),
    /// Set publishable flag
    SetPublishable(bool),
    /// Set auditable flag
    SetAuditable(bool),
    /// Set indexable flag
    SetIndexable(bool),
    /// Set strict flag
    SetStrict(bool),
}

/// DROP NODETYPE statement
///
/// ```sql
/// DROP NODETYPE 'myapp:OldType' CASCADE;
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropNodeType {
    /// Type name to drop
    pub name: String,
    /// Whether to cascade delete dependent types/nodes
    pub cascade: bool,
}

// =============================================================================
// Mixin DDL
// =============================================================================

/// CREATE MIXIN statement
///
/// ```sql
/// CREATE MIXIN 'myapp:Publishable'
///   DESCRIPTION 'Adds publishing capabilities'
///   PROPERTIES (
///     published_at Date,
///     published_by String
///   );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateMixin {
    /// Mixin name with namespace, e.g., 'myapp:Publishable'
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Icon identifier
    pub icon: Option<String>,
    /// Property definitions
    pub properties: Vec<PropertyDef>,
}

impl Default for CreateMixin {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            icon: None,
            properties: Vec::new(),
        }
    }
}

/// ALTER MIXIN statement
///
/// ```sql
/// ALTER MIXIN 'myapp:Publishable'
///   ADD PROPERTY review_status String
///   DROP PROPERTY legacy_field
///   SET DESCRIPTION = 'Updated description';
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlterMixin {
    /// Mixin name to alter
    pub name: String,
    /// Alterations to apply
    pub alterations: Vec<MixinAlteration>,
}

/// Individual alteration operation for Mixin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MixinAlteration {
    /// Add a new property
    AddProperty(PropertyDef),
    /// Drop an existing property
    DropProperty(String),
    /// Modify an existing property
    ModifyProperty(PropertyDef),
    /// Set description
    SetDescription(String),
    /// Set icon
    SetIcon(String),
}

/// DROP MIXIN statement
///
/// ```sql
/// DROP MIXIN 'myapp:OldMixin' CASCADE;
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropMixin {
    /// Mixin name to drop
    pub name: String,
    /// Whether to cascade (remove from referencing node types)
    pub cascade: bool,
}

// =============================================================================
// Archetype DDL
// =============================================================================

/// CREATE ARCHETYPE statement
///
/// ```sql
/// CREATE ARCHETYPE 'myapp:BlogPost'
///   BASE_NODE_TYPE 'myapp:Article'
///   DESCRIPTION 'Blog post archetype'
///   FIELDS (
///     title String REQUIRED,
///     body Composite
///   );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CreateArchetype {
    /// Archetype name with namespace
    pub name: String,
    /// Parent archetype to extend
    pub extends: Option<String>,
    /// Base node type this archetype is based on
    pub base_node_type: Option<String>,
    /// Human-readable title
    pub title: Option<String>,
    /// Human-readable description
    pub description: Option<String>,
    /// Icon identifier
    pub icon: Option<String>,
    /// Field definitions (similar to properties)
    pub fields: Vec<PropertyDef>,
    /// Initial content structure
    pub initial_content: Option<serde_json::Value>,
    /// View configuration
    pub view: Option<serde_json::Value>,
    /// Whether this archetype can be published
    pub publishable: bool,
}

/// ALTER ARCHETYPE statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlterArchetype {
    /// Archetype name to alter
    pub name: String,
    /// Alterations to apply
    pub alterations: Vec<ArchetypeAlteration>,
}

/// Individual alteration operation for Archetype
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ArchetypeAlteration {
    /// Add a new field
    AddField(PropertyDef),
    /// Drop an existing field
    DropField(String),
    /// Modify an existing field
    ModifyField(PropertyDef),
    /// Set description
    SetDescription(String),
    /// Set title
    SetTitle(String),
    /// Set icon
    SetIcon(String),
    /// Set base node type
    SetBaseNodeType(Option<String>),
    /// Set extends (parent archetype)
    SetExtends(Option<String>),
    /// Set publishable flag
    SetPublishable(bool),
}

/// DROP ARCHETYPE statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropArchetype {
    /// Archetype name to drop
    pub name: String,
    /// Whether to cascade delete dependent content
    pub cascade: bool,
}

// =============================================================================
// ElementType DDL
// =============================================================================

/// CREATE ELEMENTTYPE statement
///
/// ```sql
/// CREATE ELEMENTTYPE 'myapp:Paragraph'
///   DESCRIPTION 'Rich text paragraph'
///   FIELDS (
///     text String REQUIRED TRANSLATABLE,
///     style String
///   );
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CreateElementType {
    /// ElementType name with namespace
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Icon identifier
    pub icon: Option<String>,
    /// Field definitions
    pub fields: Vec<PropertyDef>,
    /// Initial content structure
    pub initial_content: Option<serde_json::Value>,
    /// Layout configuration
    pub layout: Option<serde_json::Value>,
    /// View configuration
    pub view: Option<serde_json::Value>,
    /// Whether this element type can be published
    pub publishable: bool,
}

/// ALTER ELEMENTTYPE statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlterElementType {
    /// ElementType name to alter
    pub name: String,
    /// Alterations to apply
    pub alterations: Vec<ElementTypeAlteration>,
}

/// Individual alteration operation for ElementType
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ElementTypeAlteration {
    /// Add a new field
    AddField(PropertyDef),
    /// Drop an existing field
    DropField(String),
    /// Modify an existing field
    ModifyField(PropertyDef),
    /// Set description
    SetDescription(String),
    /// Set icon
    SetIcon(String),
    /// Set publishable flag
    SetPublishable(bool),
}

/// DROP ELEMENTTYPE statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropElementType {
    /// ElementType name to drop
    pub name: String,
    /// Whether to cascade delete dependent content
    pub cascade: bool,
}
