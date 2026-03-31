//! PGQ Core Types
//!
//! Defines the fundamental types used throughout PGQ execution.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::context::PgqContext;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// SQL value type for PGQ results
///
/// Represents values that can appear in GRAPH_TABLE output columns.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    /// NULL value
    Null,
    /// Boolean
    Boolean(bool),
    /// 64-bit integer
    Integer(i64),
    /// 64-bit float
    Float(f64),
    /// UTF-8 string
    String(String),
    /// Array of values (for COLLECT aggregate)
    Array(Vec<SqlValue>),
    /// JSON value (for properties)
    Json(serde_json::Value),
}

impl SqlValue {
    /// Check if value is NULL
    pub fn is_null(&self) -> bool {
        matches!(self, SqlValue::Null)
    }

    /// Try to convert to boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SqlValue::Boolean(b) => Some(*b),
            SqlValue::Integer(i) => Some(*i != 0),
            _ => None,
        }
    }

    /// Try to convert to string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            SqlValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to convert to integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            SqlValue::Integer(i) => Some(*i),
            SqlValue::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// Try to convert to float
    pub fn as_float(&self) -> Option<f64> {
        match self {
            SqlValue::Float(f) => Some(*f),
            SqlValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl From<bool> for SqlValue {
    fn from(v: bool) -> Self {
        SqlValue::Boolean(v)
    }
}

impl From<i64> for SqlValue {
    fn from(v: i64) -> Self {
        SqlValue::Integer(v)
    }
}

impl From<f64> for SqlValue {
    fn from(v: f64) -> Self {
        SqlValue::Float(v)
    }
}

impl From<String> for SqlValue {
    fn from(v: String) -> Self {
        SqlValue::String(v)
    }
}

impl From<&str> for SqlValue {
    fn from(v: &str) -> Self {
        SqlValue::String(v.to_string())
    }
}

impl From<Option<f32>> for SqlValue {
    fn from(v: Option<f32>) -> Self {
        match v {
            Some(f) => SqlValue::Float(f as f64),
            None => SqlValue::Null,
        }
    }
}

impl From<serde_json::Value> for SqlValue {
    fn from(v: serde_json::Value) -> Self {
        SqlValue::Json(v)
    }
}

/// A single row in GRAPH_TABLE output
///
/// Contains column name -> value mappings for flat SQL output.
#[derive(Debug, Clone, Default)]
pub struct PgqRow {
    /// Column values by name
    columns: HashMap<String, SqlValue>,
    /// Column order for consistent output
    column_order: Vec<String>,
}

impl PgqRow {
    /// Create a new empty row
    pub fn new() -> Self {
        Self::default()
    }

    /// Create row with capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            columns: HashMap::with_capacity(capacity),
            column_order: Vec::with_capacity(capacity),
        }
    }

    /// Set a column value
    pub fn set(&mut self, name: impl Into<String>, value: SqlValue) {
        let name = name.into();
        if !self.columns.contains_key(&name) {
            self.column_order.push(name.clone());
        }
        self.columns.insert(name, value);
    }

    /// Get a column value
    pub fn get(&self, name: &str) -> Option<&SqlValue> {
        self.columns.get(name)
    }

    /// Get column names in order
    pub fn column_names(&self) -> &[String] {
        &self.column_order
    }

    /// Iterate over columns in order
    pub fn iter(&self) -> impl Iterator<Item = (&str, &SqlValue)> {
        self.column_order
            .iter()
            .filter_map(|name| self.columns.get(name).map(|v| (name.as_str(), v)))
    }
}

/// Information about a matched node
///
/// Stores minimal information initially; full node data is loaded lazily.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node ID
    pub id: String,
    /// Workspace containing the node
    pub workspace: String,
    /// Node type (label)
    pub node_type: String,
    /// Lazily loaded full node data
    data: Option<NodeDataCache>,
    /// True if load was attempted but node was missing/tombstone
    is_missing: bool,
}

/// Cached node data
#[derive(Debug, Clone)]
struct NodeDataCache {
    pub path: String,
    pub name: String,
    pub properties: HashMap<String, PropertyValue>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl NodeInfo {
    /// Create a new node info with minimal data
    pub fn new(id: String, workspace: String, node_type: String) -> Self {
        Self {
            id,
            workspace,
            node_type,
            data: None,
            is_missing: false,
        }
    }

    /// Create from storage node
    pub fn from_node(node: &Node, workspace: String) -> Self {
        Self {
            id: node.id.clone(),
            workspace,
            node_type: node.node_type.clone(),
            data: Some(NodeDataCache {
                path: node.path.clone(),
                name: node.name.clone(),
                properties: node.properties.clone(),
                created_at: node.created_at,
                updated_at: node.updated_at,
            }),
            is_missing: false,
        }
    }

    /// Check if full data is loaded
    pub fn is_loaded(&self) -> bool {
        self.data.is_some()
    }

    /// Check if node is missing (tombstone or deleted)
    pub fn is_missing(&self) -> bool {
        self.is_missing
    }

    /// Get path (requires data to be loaded)
    pub fn path(&self) -> Option<&str> {
        self.data.as_ref().map(|d| d.path.as_str())
    }

    /// Get name (requires data to be loaded)
    pub fn name(&self) -> Option<&str> {
        self.data.as_ref().map(|d| d.name.as_str())
    }

    /// Get property value
    pub fn get_property(&self, key: &str) -> Option<&PropertyValue> {
        self.data.as_ref().and_then(|d| d.properties.get(key))
    }

    /// Load full node data from storage
    ///
    /// Returns Ok(()) even if node is missing (tombstone/deleted).
    /// Use `is_missing()` to check if the node was found.
    pub async fn ensure_loaded<S: Storage>(
        &mut self,
        storage: &Arc<S>,
        context: &PgqContext,
    ) -> Result<()> {
        if self.data.is_some() || self.is_missing {
            return Ok(());
        }

        let node_result = storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &self.workspace,
                ),
                &self.id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))?;

        match node_result {
            Some(node) => {
                self.data = Some(NodeDataCache {
                    path: node.path,
                    name: node.name,
                    properties: node.properties,
                    created_at: node.created_at,
                    updated_at: node.updated_at,
                });
            }
            None => {
                // Node is missing (tombstone or deleted) - mark as missing but don't fail
                tracing::warn!(
                    "PGQ: Node {} is missing/tombstone (orphaned relation detected)",
                    self.id
                );
                self.is_missing = true;
            }
        }

        Ok(())
    }

    /// Get created_at timestamp (requires data to be loaded)
    pub fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.data.as_ref().and_then(|d| d.created_at)
    }

    /// Get updated_at timestamp (requires data to be loaded)
    pub fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.data.as_ref().and_then(|d| d.updated_at)
    }

    /// Get all properties as a HashMap (requires data to be loaded)
    pub fn properties(&self) -> Option<&HashMap<String, PropertyValue>> {
        self.data.as_ref().map(|d| &d.properties)
    }
}

/// Information about a matched relationship
#[derive(Debug, Clone)]
pub struct RelationInfo {
    /// Relationship type (e.g., "FOLLOWS", "similar-to")
    pub relation_type: String,
    /// Optional weight for graph algorithms
    pub weight: Option<f32>,
    /// Variable name of source node
    pub source_var: String,
    /// Variable name of target node
    pub target_var: String,
}

impl RelationInfo {
    /// Create a new relation info
    pub fn new(
        relation_type: String,
        weight: Option<f32>,
        source_var: String,
        target_var: String,
    ) -> Self {
        Self {
            relation_type,
            weight,
            source_var,
            target_var,
        }
    }
}

/// Variable bindings from pattern matching
///
/// Stores nodes and relationships matched during graph traversal.
#[derive(Debug, Clone, Default)]
pub struct VariableBinding {
    /// Bound nodes: variable name -> NodeInfo
    nodes: HashMap<String, NodeInfo>,
    /// Bound relationships: variable name -> RelationInfo
    relations: HashMap<String, RelationInfo>,
}

impl VariableBinding {
    /// Create an empty binding
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a node to a variable
    pub fn bind_node(&mut self, var: String, node: NodeInfo) {
        self.nodes.insert(var, node);
    }

    /// Bind a relationship to a variable
    pub fn bind_relation(&mut self, var: String, rel: RelationInfo) {
        self.relations.insert(var, rel);
    }

    /// Get a bound node
    pub fn get_node(&self, var: &str) -> Option<&NodeInfo> {
        self.nodes.get(var)
    }

    /// Get a mutable bound node (for lazy loading)
    pub fn get_node_mut(&mut self, var: &str) -> Option<&mut NodeInfo> {
        self.nodes.get_mut(var)
    }

    /// Get a bound relationship
    pub fn get_relation(&self, var: &str) -> Option<&RelationInfo> {
        self.relations.get(var)
    }

    /// Get all bound node variables
    pub fn node_vars(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(|s| s.as_str())
    }

    /// Get all bound relation variables
    pub fn relation_vars(&self) -> impl Iterator<Item = &str> {
        self.relations.keys().map(|s| s.as_str())
    }

    /// Check if any bound node is missing (tombstone/deleted)
    ///
    /// Returns true if at least one node in this binding is missing.
    pub fn has_missing_nodes(&self) -> bool {
        self.nodes.values().any(|node| node.is_missing())
    }

    /// Get IDs of missing nodes for logging
    pub fn missing_node_ids(&self) -> Vec<&str> {
        self.nodes
            .values()
            .filter(|node| node.is_missing())
            .map(|node| node.id.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_value_conversions() {
        assert_eq!(SqlValue::from(true), SqlValue::Boolean(true));
        assert_eq!(SqlValue::from(42i64), SqlValue::Integer(42));
        assert_eq!(SqlValue::from(3.14f64), SqlValue::Float(3.14));
        assert_eq!(SqlValue::from("hello"), SqlValue::String("hello".into()));
    }

    #[test]
    fn test_pgq_row() {
        let mut row = PgqRow::new();
        row.set("name", SqlValue::from("alice"));
        row.set("age", SqlValue::from(30i64));

        assert_eq!(row.get("name"), Some(&SqlValue::String("alice".into())));
        assert_eq!(row.get("age"), Some(&SqlValue::Integer(30)));
        assert_eq!(row.get("missing"), None);

        assert_eq!(row.column_names(), &["name", "age"]);
    }

    #[test]
    fn test_variable_binding() {
        let mut binding = VariableBinding::new();

        binding.bind_node(
            "a".into(),
            NodeInfo::new("node-1".into(), "ws".into(), "User".into()),
        );

        binding.bind_relation(
            "r".into(),
            RelationInfo::new("FOLLOWS".into(), Some(0.9), "a".into(), "b".into()),
        );

        assert!(binding.get_node("a").is_some());
        assert!(binding.get_relation("r").is_some());
        assert!(binding.get_node("missing").is_none());
    }
}
