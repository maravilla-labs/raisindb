//! RELATE/UNRELATE statement AST types
//!
//! Defines the abstract syntax tree for relationship management statements.
//!
//! # Grammar
//!
//! ```text
//! RELATE Statement:
//!   RELATE [IN BRANCH literal_string]
//!     FROM node_reference [IN WORKSPACE literal_string]
//!     TO node_reference [IN WORKSPACE literal_string]
//!     [TYPE literal_string]
//!     [WEIGHT numeric_literal]
//!   ;
//!
//! UNRELATE Statement:
//!   UNRELATE [IN BRANCH literal_string]
//!     FROM node_reference [IN WORKSPACE literal_string]
//!     TO node_reference [IN WORKSPACE literal_string]
//!     [TYPE literal_string]
//!   ;
//!
//! node_reference:
//!     path = literal_string
//!   | id = literal_string
//!   ;
//! ```
//!
//! # Examples
//!
//! ```sql
//! -- Simple relationship
//! RELATE FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged';
//!
//! -- Cross-workspace with weight
//! RELATE
//!   FROM path='/content/page' IN WORKSPACE 'main'
//!   TO path='/assets/hero.jpg' IN WORKSPACE 'media'
//!   TYPE 'references'
//!   WEIGHT 2.0;
//!
//! -- Remove relationship
//! UNRELATE FROM path='/articles/post-1' TO path='/tags/tech';
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;

/// Node reference for RELATE/UNRELATE statements
///
/// Identifies a node by either path or id
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RelateNodeReference {
    /// Reference by path (e.g., path='/content/page1')
    Path(String),
    /// Reference by ID (e.g., id='abc123')
    Id(String),
}

impl RelateNodeReference {
    /// Create a path reference
    pub fn path(p: impl Into<String>) -> Self {
        Self::Path(p.into())
    }

    /// Create an ID reference
    pub fn id(i: impl Into<String>) -> Self {
        Self::Id(i.into())
    }

    /// Get the value (path or id string)
    pub fn value(&self) -> &str {
        match self {
            Self::Path(p) => p,
            Self::Id(i) => i,
        }
    }

    /// Check if this is a path reference
    pub fn is_path(&self) -> bool {
        matches!(self, Self::Path(_))
    }
}

impl fmt::Display for RelateNodeReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(p) => write!(f, "path='{}'", p),
            Self::Id(i) => write!(f, "id='{}'", i),
        }
    }
}

/// Source or target specification with optional workspace
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelateEndpoint {
    /// Node reference (path or id)
    pub node_ref: RelateNodeReference,
    /// Optional workspace (if not specified, uses the default workspace)
    pub workspace: Option<String>,
}

impl RelateEndpoint {
    /// Create a new endpoint
    pub fn new(node_ref: RelateNodeReference, workspace: Option<String>) -> Self {
        Self {
            node_ref,
            workspace,
        }
    }

    /// Create from path
    pub fn from_path(path: impl Into<String>, workspace: Option<String>) -> Self {
        Self {
            node_ref: RelateNodeReference::Path(path.into()),
            workspace,
        }
    }

    /// Create from id
    pub fn from_id(id: impl Into<String>, workspace: Option<String>) -> Self {
        Self {
            node_ref: RelateNodeReference::Id(id.into()),
            workspace,
        }
    }
}

/// RELATE statement AST
///
/// Creates a directed relationship from source to target node
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelateStatement {
    /// Optional branch override (IN BRANCH 'x')
    pub branch: Option<String>,
    /// Source node (FROM ...)
    pub source: RelateEndpoint,
    /// Target node (TO ...)
    pub target: RelateEndpoint,
    /// Optional relationship type (TYPE 'references')
    /// Defaults to "references" if not specified
    pub relation_type: Option<String>,
    /// Optional weight for graph algorithms (WEIGHT 1.5)
    pub weight: Option<f64>,
}

impl RelateStatement {
    /// Create a new RELATE statement
    pub fn new(source: RelateEndpoint, target: RelateEndpoint) -> Self {
        Self {
            branch: None,
            source,
            target,
            relation_type: None,
            weight: None,
        }
    }

    /// Set branch override
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set relationship type
    pub fn with_type(mut self, rel_type: impl Into<String>) -> Self {
        self.relation_type = Some(rel_type.into());
        self
    }

    /// Set weight
    pub fn with_weight(mut self, weight: f64) -> Self {
        self.weight = Some(weight);
        self
    }
}

/// UNRELATE statement AST
///
/// Removes a directed relationship from source to target node
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnrelateStatement {
    /// Optional branch override (IN BRANCH 'x')
    pub branch: Option<String>,
    /// Source node (FROM ...)
    pub source: RelateEndpoint,
    /// Target node (TO ...)
    pub target: RelateEndpoint,
    /// Optional relationship type to remove (TYPE 'references')
    /// If not specified, removes any relationship between the nodes
    pub relation_type: Option<String>,
}

impl UnrelateStatement {
    /// Create a new UNRELATE statement
    pub fn new(source: RelateEndpoint, target: RelateEndpoint) -> Self {
        Self {
            branch: None,
            source,
            target,
            relation_type: None,
        }
    }

    /// Set branch override
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set relationship type filter
    pub fn with_type(mut self, rel_type: impl Into<String>) -> Self {
        self.relation_type = Some(rel_type.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relate_statement_builder() {
        let stmt = RelateStatement::new(
            RelateEndpoint::from_path("/content/page", None),
            RelateEndpoint::from_path("/assets/image", Some("media".to_string())),
        )
        .with_type("references")
        .with_weight(1.5)
        .with_branch("feature/new");

        assert_eq!(stmt.branch, Some("feature/new".to_string()));
        assert_eq!(stmt.relation_type, Some("references".to_string()));
        assert_eq!(stmt.weight, Some(1.5));
        assert!(matches!(stmt.source.node_ref, RelateNodeReference::Path(_)));
        assert_eq!(stmt.target.workspace, Some("media".to_string()));
    }

    #[test]
    fn test_unrelate_statement_builder() {
        let stmt = UnrelateStatement::new(
            RelateEndpoint::from_id("node-123", None),
            RelateEndpoint::from_id("node-456", None),
        )
        .with_type("tagged");

        assert_eq!(stmt.branch, None);
        assert_eq!(stmt.relation_type, Some("tagged".to_string()));
        assert!(matches!(stmt.source.node_ref, RelateNodeReference::Id(_)));
    }
}
