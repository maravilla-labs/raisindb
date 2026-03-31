use super::super::types::DataType;

/// Reserved schema table names (PascalCase)
/// These tables provide SQL access to schema management operations
pub const SCHEMA_TABLES: &[&str] = &["NodeTypes", "Archetypes", "ElementTypes"];

/// Check if a table name is a reserved schema table (case-insensitive)
pub fn is_schema_table(name: &str) -> bool {
    SCHEMA_TABLES.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// Enum representing the kind of schema table
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaTableKind {
    NodeTypes,
    Archetypes,
    ElementTypes,
}

impl SchemaTableKind {
    /// Try to parse a table name into a SchemaTableKind
    pub fn from_table_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "nodetypes" => Some(Self::NodeTypes),
            "archetypes" => Some(Self::Archetypes),
            "elementtypes" => Some(Self::ElementTypes),
            _ => None,
        }
    }

    /// Get the canonical table name for this schema table kind
    pub fn table_name(&self) -> &'static str {
        match self {
            Self::NodeTypes => "NodeTypes",
            Self::Archetypes => "Archetypes",
            Self::ElementTypes => "ElementTypes",
        }
    }
}

/// Table definition
#[derive(Debug, Clone)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub primary_key: Vec<String>,
    pub indexes: Vec<IndexDef>,
}

impl TableDef {
    /// Get a column by name
    pub fn get_column(&self, name: &str) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Get all column names
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

/// Column definition
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
    pub generated: Option<GeneratedExpr>,
}

impl ColumnDef {
    /// Create a non-nullable column with no generated expression
    pub fn simple(name: &str, data_type: DataType) -> Self {
        Self {
            name: name.to_string(),
            data_type,
            nullable: false,
            generated: None,
        }
    }

    /// Create a nullable column with no generated expression
    pub fn nullable(name: &str, data_type: DataType) -> Self {
        Self {
            name: name.to_string(),
            data_type,
            nullable: true,
            generated: None,
        }
    }
}

/// Generated column expression
#[derive(Debug, Clone, PartialEq)]
pub enum GeneratedExpr {
    Depth,      // DEPTH(path)
    ParentPath, // PARENT(path)
    Revision,   // __revision - current revision number
    Branch,     // __branch - branch name
    Workspace,  // __workspace - workspace name
    Locale,     // locale - resolved locale code (from translation resolution)
}

/// Index definition (for optimizer hints)
#[derive(Debug, Clone)]
pub struct IndexDef {
    pub name: String,
    pub columns: Vec<String>,
    pub index_type: IndexType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexType {
    BTree,
    PrefixRange, // For path prefix scans
    FullText,    // For text search
}
