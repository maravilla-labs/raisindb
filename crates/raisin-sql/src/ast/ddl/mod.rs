//! DDL (Data Definition Language) AST Types
//!
//! Defines AST nodes for CREATE/ALTER/DROP statements for schema management:
//! - NodeTypes
//! - Archetypes
//! - ElementTypes

mod entities;
mod properties;

pub use entities::{
    AlterArchetype, AlterElementType, AlterMixin, AlterNodeType, ArchetypeAlteration,
    CreateArchetype, CreateElementType, CreateMixin, CreateNodeType, DropArchetype,
    DropElementType, DropMixin, DropNodeType, ElementTypeAlteration, MixinAlteration,
    NodeTypeAlteration,
};
pub use properties::{
    CompoundIndexColumnDef, CompoundIndexDef, DefaultValue, IndexTypeDef, PropertyDef,
    PropertyTypeDef,
};

use serde::{Deserialize, Serialize};

/// DDL statement types for schema management
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DdlStatement {
    // NodeType operations
    CreateNodeType(CreateNodeType),
    AlterNodeType(AlterNodeType),
    DropNodeType(DropNodeType),

    // Mixin operations
    CreateMixin(CreateMixin),
    AlterMixin(AlterMixin),
    DropMixin(DropMixin),

    // Archetype operations
    CreateArchetype(CreateArchetype),
    AlterArchetype(AlterArchetype),
    DropArchetype(DropArchetype),

    // ElementType operations
    CreateElementType(CreateElementType),
    AlterElementType(AlterElementType),
    DropElementType(DropElementType),
}

impl DdlStatement {
    /// Get the schema type name being operated on
    pub fn type_name(&self) -> &str {
        match self {
            DdlStatement::CreateNodeType(c) => &c.name,
            DdlStatement::AlterNodeType(a) => &a.name,
            DdlStatement::DropNodeType(d) => &d.name,
            DdlStatement::CreateMixin(c) => &c.name,
            DdlStatement::AlterMixin(a) => &a.name,
            DdlStatement::DropMixin(d) => &d.name,
            DdlStatement::CreateArchetype(c) => &c.name,
            DdlStatement::AlterArchetype(a) => &a.name,
            DdlStatement::DropArchetype(d) => &d.name,
            DdlStatement::CreateElementType(c) => &c.name,
            DdlStatement::AlterElementType(a) => &a.name,
            DdlStatement::DropElementType(d) => &d.name,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        match self {
            DdlStatement::CreateNodeType(_) => "CREATE NODETYPE",
            DdlStatement::AlterNodeType(_) => "ALTER NODETYPE",
            DdlStatement::DropNodeType(_) => "DROP NODETYPE",
            DdlStatement::CreateMixin(_) => "CREATE MIXIN",
            DdlStatement::AlterMixin(_) => "ALTER MIXIN",
            DdlStatement::DropMixin(_) => "DROP MIXIN",
            DdlStatement::CreateArchetype(_) => "CREATE ARCHETYPE",
            DdlStatement::AlterArchetype(_) => "ALTER ARCHETYPE",
            DdlStatement::DropArchetype(_) => "DROP ARCHETYPE",
            DdlStatement::CreateElementType(_) => "CREATE ELEMENTTYPE",
            DdlStatement::AlterElementType(_) => "ALTER ELEMENTTYPE",
            DdlStatement::DropElementType(_) => "DROP ELEMENTTYPE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_nodetype_default() {
        let node_type = CreateNodeType::default();
        assert!(node_type.name.is_empty());
        assert!(node_type.extends.is_none());
        assert!(node_type.properties.is_empty());
        assert!(node_type.indexable); // default true
    }

    #[test]
    fn test_property_def_display() {
        assert_eq!(PropertyTypeDef::String.to_string(), "String");
        assert_eq!(
            PropertyTypeDef::Array {
                items: Box::new(PropertyTypeDef::String)
            }
            .to_string(),
            "Array OF String"
        );
    }

    #[test]
    fn test_ddl_statement_operation() {
        let create = DdlStatement::CreateNodeType(CreateNodeType {
            name: "test:Article".to_string(),
            ..Default::default()
        });
        assert_eq!(create.operation(), "CREATE NODETYPE");
        assert_eq!(create.type_name(), "test:Article");
    }
}
