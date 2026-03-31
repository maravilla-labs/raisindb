//! Graph pattern types for SQL/PGQ
//!
//! Defines node patterns, relationship patterns, path patterns, direction,
//! and path quantifiers used in MATCH clauses.

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::query::SourceSpan;

/// A single path pattern: nodes connected by relationships
///
/// ```sql
/// (a:User)-[:follows]->(b:User)-[:likes]->(c:Post)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathPattern {
    /// Alternating sequence of nodes and relationships
    pub elements: Vec<PatternElement>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// Element in a path pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternElement {
    /// Node pattern
    Node(NodePattern),
    /// Relationship pattern
    Relationship(RelationshipPattern),
}

/// Node pattern
///
/// ```sql
/// (n)                            -- any node
/// (n:User)                       -- with label
/// (n:User|Admin)                 -- multiple labels (OR)
/// (n:User WHERE n.active = true) -- with inline filter
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Labels (maps to node_type)
    pub labels: Vec<String>,
    /// Inline WHERE filter
    pub filter: Option<Box<Expr>>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

impl NodePattern {
    /// Create anonymous node pattern
    pub fn anonymous() -> Self {
        Self {
            variable: None,
            labels: vec![],
            filter: None,
            span: SourceSpan::empty(),
        }
    }

    /// Create node with variable
    pub fn with_var(var: impl Into<String>) -> Self {
        Self {
            variable: Some(var.into()),
            labels: vec![],
            filter: None,
            span: SourceSpan::empty(),
        }
    }
}

/// Relationship pattern
///
/// ```sql
/// -[r]->                -- any type, right direction
/// -[:follows]->         -- specific type
/// -[:follows|likes]->   -- multiple types
/// -[r:follows*2]->      -- exactly 2 hops
/// -[r:follows*1..3]->   -- 1 to 3 hops
/// <-[r]-                -- left direction
/// -[r]-                 -- any direction
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipPattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Relationship types (maps to relation_type)
    pub types: Vec<String>,
    /// Direction
    pub direction: Direction,
    /// Path quantifier for variable-length paths
    pub quantifier: Option<PathQuantifier>,
    /// Inline WHERE filter
    pub filter: Option<Box<Expr>>,
    /// Source location
    #[serde(default)]
    pub span: SourceSpan,
}

/// Relationship direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// (a)-[r]->(b) : a to b
    Right,
    /// (a)<-[r]-(b) : b to a
    Left,
    /// (a)-[r]-(b) : either direction
    Any,
}

/// Path quantifier for variable-length paths
///
/// ```sql
/// *       -- 1 to default max (10)
/// *2      -- exactly 2
/// *1..3   -- 1 to 3 inclusive
/// *2..    -- 2 to default max
/// *..5    -- 1 to 5
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathQuantifier {
    /// Minimum hops
    pub min: u32,
    /// Maximum hops (None = default max)
    pub max: Option<u32>,
}

impl PathQuantifier {
    /// Default maximum path length
    pub const DEFAULT_MAX: u32 = 10;

    /// Unbounded: *
    pub fn unbounded() -> Self {
        Self { min: 1, max: None }
    }

    /// Exact: *n
    pub fn exact(n: u32) -> Self {
        Self {
            min: n,
            max: Some(n),
        }
    }

    /// Range: *n..m
    pub fn range(min: u32, max: u32) -> Self {
        Self {
            min,
            max: Some(max),
        }
    }

    /// Get effective maximum
    pub fn effective_max(&self) -> u32 {
        self.max.unwrap_or(Self::DEFAULT_MAX)
    }
}
