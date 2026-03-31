//! COPY statement AST definitions
//!
//! Defines the Abstract Syntax Tree for node copy statements:
//! - COPY Page SET path='/content/pagea' TO path='/target/parent'
//! - COPY Page SET id='abc123' TO path='/target/parent' AS 'new-name'
//! - COPY TREE Article SET path='/source' TO id='target-parent-id'

use super::order::NodeReference;
use serde::{Deserialize, Serialize};

/// COPY statement for duplicating nodes (and optionally their descendants) to a new parent
///
/// ```sql
/// COPY Page SET path='/content/pagea' TO path='/new/parent'
/// COPY Page SET path='/content/pagea' TO path='/new/parent' AS 'copied-page'
/// COPY TREE BlogPost IN BRANCH 'feature-x' SET id='abc123' TO path='/target/parent'
/// ```
///
/// This copies the node (and all its descendants if COPY TREE) to become a child of the target parent.
/// New node IDs are generated during the copy operation. Publish state is cleared on copied nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyStatement {
    /// The table/node type name (e.g., "Page", "BlogPost")
    pub table: String,
    /// Optional branch override (IN BRANCH 'x' clause)
    /// If None, uses the default branch from execution context
    pub branch: Option<String>,
    /// The node being copied (source) - can be path or ID reference
    pub source: NodeReference,
    /// The target parent node - where to copy the node to
    pub target_parent: NodeReference,
    /// Optional new name for the copied node (AS 'name' clause)
    /// If None, uses the source node's name
    pub new_name: Option<String>,
    /// Whether to copy recursively (COPY TREE) or just the single node (COPY)
    pub recursive: bool,
}

impl CopyStatement {
    /// Create a new COPY statement (single node)
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
            new_name: None,
            recursive: false,
        }
    }

    /// Create a new COPY TREE statement (recursive)
    pub fn new_tree(
        table: impl Into<String>,
        source: NodeReference,
        target_parent: NodeReference,
    ) -> Self {
        Self {
            table: table.into(),
            branch: None,
            source,
            target_parent,
            new_name: None,
            recursive: true,
        }
    }

    /// Create a COPY statement with all options
    pub fn with_options(
        table: impl Into<String>,
        branch: Option<String>,
        source: NodeReference,
        target_parent: NodeReference,
        new_name: Option<String>,
        recursive: bool,
    ) -> Self {
        Self {
            table: table.into(),
            branch,
            source,
            target_parent,
            new_name,
            recursive,
        }
    }

    /// Get the operation kind as a string
    pub fn operation(&self) -> &'static str {
        if self.recursive {
            "COPY TREE"
        } else {
            "COPY"
        }
    }
}

impl std::fmt::Display for CopyStatement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.recursive {
            write!(f, "COPY TREE {}", self.table)?;
        } else {
            write!(f, "COPY {}", self.table)?;
        }

        // Optional IN BRANCH clause
        if let Some(branch) = &self.branch {
            write!(f, " IN BRANCH '{}'", branch)?;
        }

        write!(f, " SET {} TO {}", self.source, self.target_parent)?;

        // Optional AS clause
        if let Some(name) = &self.new_name {
            write!(f, " AS '{}'", name)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_statement_display() {
        let stmt = CopyStatement::new(
            "Page",
            NodeReference::path("/content/page1"),
            NodeReference::path("/target/parent"),
        );
        assert_eq!(
            stmt.to_string(),
            "COPY Page SET path='/content/page1' TO path='/target/parent'"
        );
    }

    #[test]
    fn test_copy_tree_statement_display() {
        let stmt = CopyStatement::new_tree(
            "Page",
            NodeReference::path("/content/page1"),
            NodeReference::path("/target/parent"),
        );
        assert_eq!(
            stmt.to_string(),
            "COPY TREE Page SET path='/content/page1' TO path='/target/parent'"
        );
    }

    #[test]
    fn test_copy_statement_with_id() {
        let stmt = CopyStatement::new(
            "BlogPost",
            NodeReference::id("abc123"),
            NodeReference::path("/content/target"),
        );
        assert_eq!(
            stmt.to_string(),
            "COPY BlogPost SET id='abc123' TO path='/content/target'"
        );
    }

    #[test]
    fn test_copy_statement_with_new_name() {
        let stmt = CopyStatement::with_options(
            "Page",
            None,
            NodeReference::path("/content/page1"),
            NodeReference::path("/target/parent"),
            Some("copied-page".to_string()),
            false,
        );
        assert_eq!(
            stmt.to_string(),
            "COPY Page SET path='/content/page1' TO path='/target/parent' AS 'copied-page'"
        );
    }

    #[test]
    fn test_copy_statement_with_branch() {
        let stmt = CopyStatement::with_options(
            "Page",
            Some("feature-x".to_string()),
            NodeReference::path("/content/page1"),
            NodeReference::id("target-id"),
            None,
            false,
        );
        assert_eq!(
            stmt.to_string(),
            "COPY Page IN BRANCH 'feature-x' SET path='/content/page1' TO id='target-id'"
        );
    }

    #[test]
    fn test_copy_tree_with_branch_and_name() {
        let stmt = CopyStatement::with_options(
            "Article",
            Some("feature-x".to_string()),
            NodeReference::path("/source/article"),
            NodeReference::id("target-parent-id"),
            Some("new-article".to_string()),
            true,
        );
        assert_eq!(
            stmt.to_string(),
            "COPY TREE Article IN BRANCH 'feature-x' SET path='/source/article' TO id='target-parent-id' AS 'new-article'"
        );
    }

    #[test]
    fn test_copy_statement_operation() {
        let copy_single = CopyStatement::new(
            "Article",
            NodeReference::path("/a"),
            NodeReference::path("/b"),
        );
        assert_eq!(copy_single.operation(), "COPY");

        let copy_tree = CopyStatement::new_tree(
            "Article",
            NodeReference::path("/a"),
            NodeReference::path("/b"),
        );
        assert_eq!(copy_tree.operation(), "COPY TREE");
    }
}
