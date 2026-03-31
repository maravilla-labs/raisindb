//! Core data types for Cypher execution

use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Result row from Cypher query execution
#[derive(Debug, Clone)]
pub struct CypherRow {
    pub columns: Vec<String>,
    pub values: Vec<PropertyValue>,
}

/// Information about a matched node
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node ID
    pub id: String,
    /// Node path in the workspace
    pub path: String,
    /// Node type (maps to Cypher label)
    pub node_type: String,
    /// Node properties
    pub properties: HashMap<String, PropertyValue>,
    /// Workspace containing this node
    pub workspace: String,
}

/// Information about a matched relationship
#[derive(Debug, Clone)]
pub struct RelationInfo {
    /// Source node variable
    pub source_var: String,
    /// Target node variable
    pub target_var: String,
    /// Relationship type
    pub relation_type: String,
    /// Relationship properties
    pub properties: HashMap<String, PropertyValue>,
}

/// Information about a variable-length path through the graph
#[derive(Debug, Clone)]
pub struct PathInfo {
    /// Sequence of nodes in the path (id, workspace)
    pub nodes: Vec<(String, String)>,
    /// Sequence of relationships in the path
    pub relationships: Vec<RelationInfo>,
    /// Length of the path (number of hops)
    pub length: usize,
}

impl PathInfo {
    /// Create a new path starting from a node
    pub fn new(start_id: String, start_workspace: String) -> Self {
        Self {
            nodes: vec![(start_id, start_workspace)],
            relationships: Vec::new(),
            length: 0,
        }
    }

    /// Extend the path with a new relationship and target node
    pub fn extend(
        &self,
        relation: RelationInfo,
        target_id: String,
        target_workspace: String,
    ) -> Self {
        let mut new_path = self.clone();
        new_path.relationships.push(relation);
        new_path.nodes.push((target_id, target_workspace));
        new_path.length += 1;
        new_path
    }

    /// Check if a node is already in the path (cycle detection)
    pub fn contains_node(&self, id: &str, workspace: &str) -> bool {
        self.nodes
            .iter()
            .any(|(node_id, node_workspace)| node_id == id && node_workspace == workspace)
    }
}

/// Variable bindings during query execution
///
/// Tracks which Cypher variables are bound to which nodes/relationships
#[derive(Debug, Clone)]
pub struct VariableBinding {
    /// Node variable bindings (variable name -> node info)
    pub nodes: HashMap<String, NodeInfo>,
    /// Relationship variable bindings (variable name -> relation info)
    pub relationships: HashMap<String, RelationInfo>,
}

impl VariableBinding {
    /// Create empty bindings
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            relationships: HashMap::new(),
        }
    }

    /// Bind a node variable
    pub fn bind_node(&mut self, var: String, node: NodeInfo) {
        self.nodes.insert(var, node);
    }

    /// Bind a relationship variable
    pub fn bind_relation(&mut self, var: String, relation: RelationInfo) {
        self.relationships.insert(var, relation);
    }

    /// Get a node by variable name
    pub fn get_node(&self, var: &str) -> Option<&NodeInfo> {
        self.nodes.get(var)
    }

    /// Get a relationship by variable name
    pub fn get_relation(&self, var: &str) -> Option<&RelationInfo> {
        self.relationships.get(var)
    }

    /// Check if a node variable is bound
    pub fn has_node(&self, var: &str) -> bool {
        self.nodes.contains_key(var)
    }

    /// Merge another binding into this one
    pub fn merge(&mut self, other: VariableBinding) {
        self.nodes.extend(other.nodes);
        self.relationships.extend(other.relationships);
    }

    /// Clone and extend with new bindings
    pub fn extend_with(&self, var: String, node: NodeInfo) -> Self {
        let mut new_binding = self.clone();
        new_binding.bind_node(var, node);
        new_binding
    }
}

impl Default for VariableBinding {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution context for Cypher queries
#[derive(Debug, Clone)]
pub struct CypherContext {
    /// Workspace ID
    pub workspace_id: String,
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Revision (for time-travel queries)
    pub revision: Option<raisin_hlc::HLC>,
    /// Static parameter map supplied by the caller
    pub parameters: Arc<HashMap<String, PropertyValue>>,
}

impl CypherContext {
    /// Create a new execution context
    pub fn new(
        workspace_id: String,
        tenant_id: String,
        repo_id: String,
        branch: String,
        revision: Option<raisin_hlc::HLC>,
    ) -> Self {
        Self {
            workspace_id,
            tenant_id,
            repo_id,
            branch,
            revision,
            parameters: Arc::new(HashMap::new()),
        }
    }

    /// Attach query parameters to the context
    pub fn with_parameters(mut self, parameters: HashMap<String, PropertyValue>) -> Self {
        self.parameters = Arc::new(parameters);
        self
    }
}
