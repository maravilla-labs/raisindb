//! Schema object table definitions (NodeTypes, Archetypes, ElementTypes)

use super::super::types::{ColumnDef, GeneratedExpr, TableDef};
use crate::analyzer::types::DataType;

/// Create the NodeTypes schema table definition
///
/// This table provides SQL access to NodeType CRUD operations.
/// Columns match the NodeType model fields.
pub(crate) fn node_types_table() -> TableDef {
    TableDef {
        name: "NodeTypes".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "name".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "strict".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "extends".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "mixins".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "overrides".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "description".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "icon".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "version".into(),
                data_type: DataType::Int,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "properties".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "allowed_children".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "required_nodes".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "initial_structure".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "versionable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "publishable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "auditable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "indexable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "index_types".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "created_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "updated_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_by".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "previous_version".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "__branch".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Branch),
            },
        ],
        primary_key: vec!["name".to_string()],
        indexes: vec![],
    }
}

/// Create the Archetypes schema table definition
///
/// This table provides SQL access to Archetype CRUD operations.
/// Columns match the Archetype model fields.
pub(crate) fn archetypes_table() -> TableDef {
    TableDef {
        name: "Archetypes".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "name".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "extends".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "icon".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "title".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "description".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "base_node_type".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "fields".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "initial_content".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "view".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            // Read-side exposure of the archetype's `meta` bag (meta.editor etc.).
            // Populated by the schema-table read path; the write path ignores it.
            ColumnDef {
                name: "meta".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "version".into(),
                data_type: DataType::Int,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "created_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "updated_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_by".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "publishable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "previous_version".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "__branch".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Branch),
            },
        ],
        primary_key: vec!["name".to_string()],
        indexes: vec![],
    }
}

/// Create the ElementTypes schema table definition
///
/// This table provides SQL access to ElementType CRUD operations.
/// Columns match the ElementType model fields.
pub(crate) fn element_types_table() -> TableDef {
    TableDef {
        name: "ElementTypes".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "name".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            // Read-side exposure of the element type's `extends` (e.g. studio:Component)
            // so callers can resolve inherited fields. Write path ignores it.
            ColumnDef {
                name: "extends".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "icon".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "description".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "fields".into(),
                data_type: DataType::JsonB,
                nullable: false,
                generated: None,
            },
            // Read-side exposure of the element type's `meta` bag (meta.hidden,
            // meta.policies, meta.editor). Write path ignores it.
            ColumnDef {
                name: "meta".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "initial_content".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "layout".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "view".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "version".into(),
                data_type: DataType::Int,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "created_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "updated_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_at".into(),
                data_type: DataType::TimestampTz,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "published_by".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "publishable".into(),
                data_type: DataType::Boolean,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "previous_version".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "__branch".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Branch),
            },
        ],
        primary_key: vec!["name".to_string()],
        indexes: vec![],
    }
}

/// Create the Workspaces schema table definition.
///
/// READ-ONLY: workspaces are created and altered through the management API and
/// package install, never DDL, so the DML executor rejects writes here.
///
/// The columns that earn this table its existence are `allowed_node_types` and
/// `allowed_root_node_types` — the coarse, server-enforced containment rule.
/// Without them a server-side function can see which archetypes exist
/// (`Archetypes`) and what a parent accepts (`NodeTypes.allowed_children`) but
/// not what the workspace itself permits, so it cannot reproduce the editor's
/// create menu. `config` and `initial_structure` come through as JSONB.
pub(crate) fn workspaces_table() -> TableDef {
    TableDef {
        name: "Workspaces".to_string(),
        columns: vec![
            ColumnDef {
                name: "name".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "description".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "allowed_node_types".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "allowed_root_node_types".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "depends_on".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "initial_structure".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "config".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "created_at".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "updated_at".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
        ],
        primary_key: vec!["name".to_string()],
        indexes: vec![],
    }
}
