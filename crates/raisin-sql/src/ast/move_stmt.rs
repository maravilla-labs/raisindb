//! MOVE statement AST definitions
//!
//! Defines the Abstract Syntax Tree for node move statements:
//! - MOVE Page SET path='/content/pagea' TO path='/target/parent'
//! - MOVE BlogPost SET id='abc123' TO path='/target/parent'
//! - MOVE Article SET path='/source' TO id='target-parent-id'

use super::order::NodeReference;
use serde::{Deserialize, Serialize};

/// MOVE statement for relocating nodes (and their descendants) to a new parent
///
/// ```sql
/// MOVE Page SET path='/content/pagea' TO path='/new/parent'
/// MOVE BlogPost IN BRANCH 'feature-x' SET id='abc123' TO path='/target/parent'
/// ```
///
/// This moves the node (and all its descendants) to become a child of the target parent.
/// Node IDs are preserved during the move operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveStatement {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// Optional branch override (IN BRANCH 'x' clause)
    /// If None, uses the default branch from execution context
    pub branch: Option<String>,
    /// The node being moved (source) - can be path or ID reference
    pub source: NodeReference,
    /// The target parent node - where to move the node to
    pub target_parent: NodeReference,
}

impl MoveStatement {
    /// Create a new MOVE statement
    pub fn new(
        table: impl Into<String>,
        source: NodeReference,
        target_parent: NodeReference,
    ) -> Self {
        Self {
            table: table.into(),
            branch: None,
            source,
            target_parent,
        }
    }

    /// Create a new MOVE statement with branch override
    pub fn with_branch(
        table: impl Into<String>,
        branch: Option<String>,
        source: NodeReference,
        target_parent: NodeReference,
    ) -> Self {
        Self {
            table: table.into(),
            branch,
            source,
            target_parent,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        "MOVE"
    }
}

impl std::fmt::Display for MoveStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MOVE {}", self.table)?;

        // Optional IN BRANCH clause
        if let Some(branch) = &self.branch {
            write!(f, " IN BRANCH '{}'", branch)?;
        }

        write!(f, " SET {} TO {}", self.source, self.target_parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_statement_display() {
        let stmt = MoveStatement::new(
            "Page",
            NodeReference::path("/content/page1"),
            NodeReference::path("/target/parent"),
        );
        assert_eq!(
            stmt.to_string(),
            "MOVE Page SET path='/content/page1' TO path='/target/parent'"
        );
    }

    #[test]
    fn test_move_statement_with_id() {
        let stmt = MoveStatement::new(
            "BlogPost",
            NodeReference::id("abc123"),
            NodeReference::path("/content/target"),
        );
        assert_eq!(
            stmt.to_string(),
            "MOVE BlogPost SET id='abc123' TO path='/content/target'"
        );
    }

    #[test]
    fn test_move_statement_mixed_refs() {
        let stmt = MoveStatement::new(
            "Article",
            NodeReference::path("/source/article"),
            NodeReference::id("target-parent-id"),
        );
        assert_eq!(
            stmt.to_string(),
            "MOVE Article SET path='/source/article' TO id='target-parent-id'"
        );
    }

    #[test]
    fn test_move_statement_operation() {
        let stmt = MoveStatement::new(
            "Article",
            NodeReference::path("/a"),
            NodeReference::path("/b"),
        );
        assert_eq!(stmt.operation(), "MOVE");
    }
}
