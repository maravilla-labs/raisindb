mod schema_tables;
mod static_catalog;
mod types;

// Re-export all public API items to preserve the original module interface
pub use self::static_catalog::StaticCatalog;
pub use self::types::{
    is_schema_table, ColumnDef, GeneratedExpr, IndexDef, IndexType, SchemaTableKind, TableDef,
    SCHEMA_TABLES,
};

/// Convert a workspace name to a SQL-safe CamelCase table name
///
/// This function transforms workspace names with special characters into valid
/// SQL identifiers by:
/// 1. Splitting on special characters (`:`, `-`, `_`, spaces)
/// 2. Capitalizing the first letter of each part
/// 3. Joining parts together without separators
///
/// # Examples
///
/// ```
/// # use raisin_sql::analyzer::catalog::workspace_to_table_name;
/// assert_eq!(workspace_to_table_name("raisin:access_control"), "RaisinAccessControl");
/// assert_eq!(workspace_to_table_name("raisin:user"), "RaisinUser");
/// assert_eq!(workspace_to_table_name("my-workspace"), "MyWorkspace");
/// assert_eq!(workspace_to_table_name("default"), "Default");
/// ```
pub fn workspace_to_table_name(workspace: &str) -> String {
    workspace
        .split(|c: char| c == ':' || c == '-' || c == '_' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
}

/// Trait for schema catalog
pub trait Catalog: Send + Sync {
    fn get_table(&self, name: &str) -> Option<&TableDef>;
    fn list_tables(&self) -> Vec<&str>;

    /// Check if a name refers to a workspace (default: false)
    fn is_workspace(&self, _name: &str) -> bool {
        false
    }

    /// Get table schema for a workspace (creates it dynamically if needed)
    fn get_workspace_table(&self, _workspace_name: &str) -> Option<TableDef> {
        None
    }

    /// Resolve a table name to its workspace name
    ///
    /// Returns the original workspace name for a given table name.
    /// Default implementation returns None.
    fn resolve_workspace_name(&self, _table_name: &str) -> Option<String> {
        None
    }

    /// Get embedding dimensions if vector embeddings are enabled
    /// Returns None if embeddings are not configured for this catalog
    fn embedding_dimensions(&self) -> Option<usize> {
        None
    }

    /// Clone the catalog into a Box (required for trait object cloning)
    fn clone_box(&self) -> Box<dyn Catalog>;
}

#[cfg(test)]
mod tests;
