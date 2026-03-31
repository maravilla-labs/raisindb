//! ORDER statement AST definitions
//!
//! Defines the Abstract Syntax Tree for node ordering statements:
//! - ORDER Page SET path='/content/pagea' ABOVE path='/content/pageb'
//! - ORDER BlogPost SET id='abc123' BELOW path='/content/target'
//! - ORDER Article SET path='/source' BELOW id='xyz789'

use serde::{Deserialize, Serialize};

/// Reference to a node by either path or ID
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeReference {
    /// Reference by path: path='/content/page'
    Path(String),
    /// Reference by ID: id='abc123'
    Id(String),
}

impl NodeReference {
    /// Create a path reference
    pub fn path(path: impl Into<String>) -> Self {
        NodeReference::Path(path.into())
    }

    /// Create an ID reference
    pub fn id(id: impl Into<String>) -> Self {
        NodeReference::Id(id.into())
    }

    /// Get the reference type as a string
    pub fn reference_type(&self) -> &'static str {
        match self {
            NodeReference::Path(_) => "path",
            NodeReference::Id(_) => "id",
        }
    }

    /// Get the value (path or id string)
    pub fn value(&self) -> &str {
        match self {
            NodeReference::Path(p) => p,
            NodeReference::Id(i) => i,
        }
    }

    /// Check if this is a path reference
    pub fn is_path(&self) -> bool {
        matches!(self, NodeReference::Path(_))
    }

    /// Check if this is an ID reference
    pub fn is_id(&self) -> bool {
        matches!(self, NodeReference::Id(_))
    }
}

/// Position directive for ordering - where to place source relative to target
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderPosition {
    /// Place source ABOVE target (before in sibling order)
    /// Maps to NodeService::move_child_before
    Above,
    /// Place source BELOW target (after in sibling order)
    /// Maps to NodeService::move_child_after
    Below,
}

impl OrderPosition {
    /// Convert to the position string used by NodeService
    pub fn as_position_str(&self) -> &'static str {
        match self {
            OrderPosition::Above => "before",
            OrderPosition::Below => "after",
        }
    }
}

/// ORDER statement for node sibling positioning
///
/// ```sql
/// ORDER Page SET path='/content/pagea' ABOVE path='/content/pageb'
/// ORDER BlogPost IN BRANCH 'feature-x' SET id='abc123' BELOW path='/content/target'
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderStatement {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// Optional branch override (IN BRANCH 'x' clause)
    /// If None, uses the default branch from execution context
    pub branch: Option<String>,
    /// The node being moved (source)
    pub source: NodeReference,
    /// The positioning directive (ABOVE or BELOW)
    pub position: OrderPosition,
    /// The target node to position relative to
    pub target: NodeReference,
}

impl OrderStatement {
    /// Create a new ORDER statement
    pub fn new(
        table: impl Into<String>,
        source: NodeReference,
        position: OrderPosition,
        target: NodeReference,
    ) -> Self {
        Self {
            table: table.into(),
            branch: None,
            source,
            position,
            target,
        }
    }

    /// Create a new ORDER statement with branch override
    pub fn with_branch(
        table: impl Into<String>,
        branch: Option<String>,
        source: NodeReference,
        position: OrderPosition,
        target: NodeReference,
    ) -> Self {
        Self {
            table: table.into(),
            branch,
            source,
            position,
            target,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        "ORDER"
    }
}

impl std::fmt::Display for NodeReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeReference::Path(p) => write!(f, "path='{}'", p),
            NodeReference::Id(i) => write!(f, "id='{}'", i),
        }
    }
}

impl std::fmt::Display for OrderPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderPosition::Above => write!(f, "ABOVE"),
            OrderPosition::Below => write!(f, "BELOW"),
        }
    }
}

impl std::fmt::Display for OrderStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ORDER {}", self.table)?;

        // Optional IN BRANCH clause
        if let Some(branch) = &self.branch {
            write!(f, " IN BRANCH '{}'", branch)?;
        }

        write!(f, " SET {} {} {}", self.source, self.position, self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_reference_path() {
        let node_ref = NodeReference::path("/content/page1");
        assert!(node_ref.is_path());
        assert!(!node_ref.is_id());
        assert_eq!(node_ref.value(), "/content/page1");
        assert_eq!(node_ref.reference_type(), "path");
        assert_eq!(node_ref.to_string(), "path='/content/page1'");
    }

    #[test]
    fn test_node_reference_id() {
        let node_ref = NodeReference::id("abc123");
        assert!(node_ref.is_id());
        assert!(!node_ref.is_path());
        assert_eq!(node_ref.value(), "abc123");
        assert_eq!(node_ref.reference_type(), "id");
        assert_eq!(node_ref.to_string(), "id='abc123'");
    }

    #[test]
    fn test_order_position_above() {
        let pos = OrderPosition::Above;
        assert_eq!(pos.as_position_str(), "before");
        assert_eq!(pos.to_string(), "ABOVE");
    }

    #[test]
    fn test_order_position_below() {
        let pos = OrderPosition::Below;
        assert_eq!(pos.as_position_str(), "after");
        assert_eq!(pos.to_string(), "BELOW");
    }

    #[test]
    fn test_order_statement_display() {
        let stmt = OrderStatement::new(
            "Page",
            NodeReference::path("/content/page1"),
            OrderPosition::Above,
            NodeReference::path("/content/page2"),
        );
        assert_eq!(
            stmt.to_string(),
            "ORDER Page SET path='/content/page1' ABOVE path='/content/page2'"
        );
    }

    #[test]
    fn test_order_statement_mixed_refs() {
        let stmt = OrderStatement::new(
            "BlogPost",
            NodeReference::id("abc123"),
            OrderPosition::Below,
            NodeReference::path("/content/target"),
        );
        assert_eq!(
            stmt.to_string(),
            "ORDER BlogPost SET id='abc123' BELOW path='/content/target'"
        );
    }

    #[test]
    fn test_order_statement_operation() {
        let stmt = OrderStatement::new(
            "Article",
            NodeReference::path("/a"),
            OrderPosition::Above,
            NodeReference::path("/b"),
        );
        assert_eq!(stmt.operation(), "ORDER");
    }
}
