//! Schema table definitions for the SQL catalog
//!
//! Provides table schemas for nodes, NodeTypes, Archetypes, ElementTypes,
//! and workspace-aware tables.

mod nodes_table;
mod schema_object_tables;

use super::types::{SchemaTableKind, TableDef};

/// Get schema table definition by kind
pub fn get_schema_table(kind: SchemaTableKind) -> TableDef {
    match kind {
        SchemaTableKind::NodeTypes => schema_object_tables::node_types_table(),
        SchemaTableKind::Archetypes => schema_object_tables::archetypes_table(),
        SchemaTableKind::ElementTypes => schema_object_tables::element_types_table(),
    }
}

pub(super) use nodes_table::default_nodes_table;
pub(super) use nodes_table::workspace_table;
