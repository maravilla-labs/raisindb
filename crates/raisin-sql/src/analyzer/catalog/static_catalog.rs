use std::collections::HashMap;

use super::schema_tables;
use super::types::{ColumnDef, SchemaTableKind, TableDef};
use super::{workspace_to_table_name, Catalog};
use crate::analyzer::types::DataType;

/// Static catalog implementation seeded from Rust structs
#[derive(Debug, Clone)]
pub struct StaticCatalog {
    tables: HashMap<String, TableDef>,
    /// Workspaces that are registered as tables
    workspaces: Vec<String>,
    /// Embedding dimensions for vector similarity search (if enabled)
    embedding_dimensions: Option<usize>,
    /// Bidirectional mapping: workspace name -> CamelCase table name
    workspace_to_table: HashMap<String, String>,
    /// Bidirectional mapping: CamelCase table name -> workspace name
    table_to_workspace: HashMap<String, String>,
}

impl StaticCatalog {
    /// Create catalog with the default RaisinDB nodes schema
    pub fn default_nodes_schema() -> Self {
        let mut tables = HashMap::new();
        tables.insert("nodes".to_string(), schema_tables::default_nodes_table());

        // Register schema tables for NodeTypes, Archetypes, ElementTypes
        tables.insert(
            "NodeTypes".to_string(),
            schema_tables::get_schema_table(SchemaTableKind::NodeTypes),
        );
        tables.insert(
            "Archetypes".to_string(),
            schema_tables::get_schema_table(SchemaTableKind::Archetypes),
        );
        tables.insert(
            "ElementTypes".to_string(),
            schema_tables::get_schema_table(SchemaTableKind::ElementTypes),
        );

        let mut workspace_to_table = HashMap::new();
        let mut table_to_workspace = HashMap::new();

        // Register default workspace with mapping
        let default_workspace = "default".to_string();
        let default_table_name = workspace_to_table_name(&default_workspace);
        workspace_to_table.insert(default_workspace.clone(), default_table_name.clone());
        table_to_workspace.insert(default_table_name, default_workspace.clone());

        StaticCatalog {
            tables,
            workspaces: vec![default_workspace],
            embedding_dimensions: None,
            workspace_to_table,
            table_to_workspace,
        }
    }

    /// Add embedding column to the nodes table schema
    ///
    /// This method adds a virtual `embedding` column with the specified dimensions
    /// to support vector similarity search queries. The column is nullable since not
    /// all nodes have embeddings.
    ///
    /// # Arguments
    ///
    /// * `dimensions` - The embedding vector dimensions (e.g., 1536 for OpenAI text-embedding-ada-002)
    ///
    /// # Example
    ///
    /// ```rust
    /// use raisin_sql::analyzer::catalog::StaticCatalog;
    ///
    /// // Create catalog with embedding support for 1536-dimensional vectors
    /// let catalog = StaticCatalog::default_nodes_schema()
    ///     .with_embedding_column(1536);
    /// ```
    pub fn with_embedding_column(mut self, dimensions: usize) -> Self {
        // Store dimensions for workspace table generation
        self.embedding_dimensions = Some(dimensions);

        // Add embedding column to the nodes table
        if let Some(table) = self.tables.get_mut("nodes") {
            table.columns.push(ColumnDef {
                name: "embedding".into(),
                data_type: DataType::Vector(dimensions),
                nullable: true, // Not all nodes have embeddings
                generated: None,
            });
        }
        self
    }

    /// Get schema table definition by kind
    pub fn get_schema_table(kind: SchemaTableKind) -> TableDef {
        schema_tables::get_schema_table(kind)
    }

    /// Register a workspace as a queryable table
    pub fn register_workspace(&mut self, workspace_name: String) {
        if !self.workspaces.contains(&workspace_name) {
            // Generate CamelCase table name from workspace name
            let table_name = workspace_to_table_name(&workspace_name);

            // Populate bidirectional mapping
            self.workspace_to_table
                .insert(workspace_name.clone(), table_name.clone());
            self.table_to_workspace
                .insert(table_name, workspace_name.clone());

            self.workspaces.push(workspace_name);
        }
    }

    /// Create an empty catalog
    pub fn new() -> Self {
        StaticCatalog {
            tables: HashMap::new(),
            workspaces: Vec::new(),
            embedding_dimensions: None,
            workspace_to_table: HashMap::new(),
            table_to_workspace: HashMap::new(),
        }
    }

    /// Add a table to the catalog
    pub fn add_table(&mut self, table: TableDef) {
        self.tables.insert(table.name.clone(), table);
    }

    /// Get list of registered workspaces
    pub fn workspaces(&self) -> &[String] {
        &self.workspaces
    }
}

impl Default for StaticCatalog {
    fn default() -> Self {
        Self::default_nodes_schema()
    }
}

impl Catalog for StaticCatalog {
    fn get_table(&self, name: &str) -> Option<&TableDef> {
        // First check static tables
        if let Some(table) = self.tables.get(name) {
            return Some(table);
        }

        // Check if it's a registered workspace
        // We can't return a reference to a dynamically created table
        // So workspace tables need special handling in the semantic analyzer
        None
    }

    fn list_tables(&self) -> Vec<&str> {
        let mut tables: Vec<&str> = self.tables.keys().map(|s| s.as_str()).collect();
        tables.extend(self.workspaces.iter().map(|s| s.as_str()));
        tables
    }

    fn is_workspace(&self, name: &str) -> bool {
        // Check if it's a mapped table name (CamelCase)
        self.table_to_workspace.contains_key(name)
            // Or check if it's a direct workspace name (backward compat)
            || self.workspaces.contains(&name.to_string())
    }

    fn get_workspace_table(&self, table_name: &str) -> Option<TableDef> {
        // First, try to resolve as a mapped table name (CamelCase -> workspace)
        let _workspace_name = self
            .table_to_workspace
            .get(table_name)
            .map(|s| s.as_str())
            .or_else(|| {
                // Fallback: check if it's already a workspace name (backward compat)
                if self.workspaces.contains(&table_name.to_string()) {
                    Some(table_name)
                } else {
                    None
                }
            })?;

        // Validate workspace exists and return table definition
        // Use table_name for schema, workspace info is in TableRef.workspace
        Some(schema_tables::workspace_table(
            table_name,
            self.embedding_dimensions,
        ))
    }

    fn resolve_workspace_name(&self, table_name: &str) -> Option<String> {
        // First, try to resolve as a mapped table name (CamelCase -> workspace)
        self.table_to_workspace
            .get(table_name)
            .cloned()
            .or_else(|| {
                // Fallback: check if it's already a workspace name (backward compat)
                if self.workspaces.contains(&table_name.to_string()) {
                    Some(table_name.to_string())
                } else {
                    None
                }
            })
    }

    fn embedding_dimensions(&self) -> Option<usize> {
        self.embedding_dimensions
    }

    fn clone_box(&self) -> Box<dyn Catalog> {
        Box::new(self.clone())
    }
}

impl StaticCatalog {
    /// Check if a name refers to a workspace or a mapped table name
    pub fn is_workspace(&self, name: &str) -> bool {
        // Check if it's a mapped table name (CamelCase)
        self.table_to_workspace.contains_key(name)
            // Or check if it's a direct workspace name (backward compat)
            || self.workspaces.contains(&name.to_string())
    }

    /// Resolve a table name to its workspace name
    ///
    /// Returns the original workspace name for a given table name.
    /// If the table name is already a workspace name, returns it as-is.
    pub fn resolve_workspace_name(&self, table_name: &str) -> Option<String> {
        // First, try to resolve as a mapped table name (CamelCase -> workspace)
        self.table_to_workspace
            .get(table_name)
            .cloned()
            .or_else(|| {
                // Fallback: check if it's already a workspace name (backward compat)
                if self.workspaces.contains(&table_name.to_string()) {
                    Some(table_name.to_string())
                } else {
                    None
                }
            })
    }

    /// Get table schema for a workspace (creates it dynamically)
    pub fn get_workspace_table(&self, table_name: &str) -> Option<TableDef> {
        // First, try to resolve as a mapped table name (CamelCase -> workspace)
        let _workspace_name = self
            .table_to_workspace
            .get(table_name)
            .map(|s| s.as_str())
            .or_else(|| {
                // Fallback: check if it's already a workspace name (backward compat)
                if self.is_workspace(table_name) {
                    Some(table_name)
                } else {
                    None
                }
            })?;

        // Validate workspace exists and return table definition
        // Use table_name for schema, workspace info is in TableRef.workspace
        Some(schema_tables::workspace_table(
            table_name,
            self.embedding_dimensions,
        ))
    }
}
