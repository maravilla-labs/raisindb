//! Node table definitions (default nodes and workspace-aware tables)

use super::super::types::{ColumnDef, GeneratedExpr, IndexDef, IndexType, TableDef};
use crate::analyzer::types::DataType;

/// Create the default nodes table definition matching the Node model
pub(crate) fn default_nodes_table() -> TableDef {
    TableDef {
        name: "nodes".to_string(),
        columns: vec![
            ColumnDef {
                name: "id".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "path".into(),
                data_type: DataType::Path,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "name".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "node_type".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "archetype".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "properties".into(),
                data_type: DataType::JsonB,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "parent_name".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "version".into(),
                data_type: DataType::Int,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "created_at".into(),
                data_type: DataType::TimestampTz,
                nullable: false,
                generated: None,
            },
            ColumnDef {
                name: "updated_at".into(),
                data_type: DataType::TimestampTz,
                nullable: false,
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
                name: "updated_by".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "created_by".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "translations".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "owner_id".into(),
                data_type: DataType::Text,
                nullable: true,
                generated: None,
            },
            ColumnDef {
                name: "relations".into(),
                data_type: DataType::JsonB,
                nullable: true,
                generated: None,
            },
            // Generated virtual columns
            ColumnDef {
                name: "parent_path".into(),
                data_type: DataType::Path,
                nullable: true,
                generated: Some(GeneratedExpr::ParentPath),
            },
            ColumnDef {
                name: "depth".into(),
                data_type: DataType::Int,
                nullable: false,
                generated: Some(GeneratedExpr::Depth),
            },
            ColumnDef {
                name: "__revision".into(),
                data_type: DataType::BigInt,
                nullable: true,
                generated: Some(GeneratedExpr::Revision),
            },
            ColumnDef {
                name: "__branch".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Branch),
            },
            ColumnDef {
                name: "__workspace".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Workspace),
            },
            ColumnDef {
                name: "locale".into(),
                data_type: DataType::Text,
                nullable: false,
                generated: Some(GeneratedExpr::Locale),
            },
        ],
        primary_key: vec!["path".into()],
        indexes: vec![
            IndexDef {
                name: "idx_path_prefix".into(),
                columns: vec!["path".into()],
                index_type: IndexType::PrefixRange,
            },
            IndexDef {
                name: "idx_depth".into(),
                columns: vec!["depth".into()],
                index_type: IndexType::BTree,
            },
        ],
    }
}

/// Create a workspace-aware table definition for a specific workspace
///
/// # Arguments
///
/// * `table_name` - The table name to use in the schema (e.g., "RaisinAccessControl")
/// * `embedding_dimensions` - Optional embedding dimensions for vector search
pub(crate) fn workspace_table(table_name: &str, embedding_dimensions: Option<usize>) -> TableDef {
    let mut columns = vec![
        ColumnDef {
            name: "id".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "path".into(),
            data_type: DataType::Path,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "name".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "node_type".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "archetype".into(),
            data_type: DataType::Text,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "properties".into(),
            data_type: DataType::JsonB,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "parent_name".into(),
            data_type: DataType::Text,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "version".into(),
            data_type: DataType::Int,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "created_at".into(),
            data_type: DataType::TimestampTz,
            nullable: false,
            generated: None,
        },
        ColumnDef {
            name: "updated_at".into(),
            data_type: DataType::TimestampTz,
            nullable: false,
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
            name: "updated_by".into(),
            data_type: DataType::Text,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "created_by".into(),
            data_type: DataType::Text,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "translations".into(),
            data_type: DataType::JsonB,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "owner_id".into(),
            data_type: DataType::Text,
            nullable: true,
            generated: None,
        },
        ColumnDef {
            name: "relations".into(),
            data_type: DataType::JsonB,
            nullable: true,
            generated: None,
        },
        // Generated virtual columns
        ColumnDef {
            name: "parent_path".into(),
            data_type: DataType::Path,
            nullable: true,
            generated: Some(GeneratedExpr::ParentPath),
        },
        ColumnDef {
            name: "depth".into(),
            data_type: DataType::Int,
            nullable: false,
            generated: Some(GeneratedExpr::Depth),
        },
        ColumnDef {
            name: "__revision".into(),
            data_type: DataType::BigInt,
            nullable: true,
            generated: Some(GeneratedExpr::Revision),
        },
        ColumnDef {
            name: "__branch".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: Some(GeneratedExpr::Branch),
        },
        ColumnDef {
            name: "__workspace".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: Some(GeneratedExpr::Workspace),
        },
        ColumnDef {
            name: "locale".into(),
            data_type: DataType::Text,
            nullable: false,
            generated: Some(GeneratedExpr::Locale),
        },
    ];

    // Add embedding column if dimensions are specified
    if let Some(dimensions) = embedding_dimensions {
        columns.push(ColumnDef {
            name: "embedding".into(),
            data_type: DataType::Vector(dimensions),
            nullable: true,
            generated: None,
        });
    }

    TableDef {
        name: table_name.to_string(),
        columns,
        primary_key: vec!["path".into()],
        indexes: vec![
            IndexDef {
                name: "idx_path_prefix".into(),
                columns: vec!["path".into()],
                index_type: IndexType::PrefixRange,
            },
            IndexDef {
                name: "idx_depth".into(),
                columns: vec!["depth".into()],
                index_type: IndexType::BTree,
            },
        ],
    }
}
