// SPDX-License-Identifier: BSL-1.1

//! Graph pattern AST nodes

use super::expr::Expr;
use serde::Serialize;
use std::fmt;

/// A complete graph pattern with optional WHERE clause
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphPattern {
    pub patterns: Vec<PathPattern>,
    pub where_clause: Option<Expr>,
}

impl GraphPattern {
    /// Create a new graph pattern
    pub fn new(patterns: Vec<PathPattern>) -> Self {
        Self {
            patterns,
            where_clause: None,
        }
    }

    /// Create a graph pattern with WHERE clause
    pub fn with_where(patterns: Vec<PathPattern>, where_clause: Expr) -> Self {
        Self {
            patterns,
            where_clause: Some(where_clause),
        }
    }
}

/// A path pattern (potentially with a variable)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathPattern {
    /// Optional variable name for the entire path
    pub variable: Option<String>,
    /// Elements in the path (alternating nodes and relationships)
    pub elements: Vec<PatternElement>,
}

impl PathPattern {
    /// Create a new path pattern
    pub fn new(elements: Vec<PatternElement>) -> Self {
        Self {
            variable: None,
            elements,
        }
    }

    /// Create a path pattern with a variable
    pub fn with_variable(variable: String, elements: Vec<PatternElement>) -> Self {
        Self {
            variable: Some(variable),
            elements,
        }
    }
}

/// Element in a path pattern (node or relationship)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum PatternElement {
    Node(NodePattern),
    Relationship(RelPattern),
}

/// Node pattern: (variable:Label {properties})
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodePattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Node labels
    pub labels: Vec<String>,
    /// Property constraints as a map expression
    pub properties: Option<Vec<(String, Expr)>>,
    /// Inline WHERE clause (Cypher 10 feature)
    pub where_clause: Option<Expr>,
}

impl NodePattern {
    /// Create an empty node pattern ()
    pub fn empty() -> Self {
        Self {
            variable: None,
            labels: Vec::new(),
            properties: None,
            where_clause: None,
        }
    }

    /// Create a node with variable
    pub fn with_variable(variable: impl Into<String>) -> Self {
        Self {
            variable: Some(variable.into()),
            labels: Vec::new(),
            properties: None,
            where_clause: None,
        }
    }

    /// Add labels to the node
    pub fn with_labels(mut self, labels: Vec<String>) -> Self {
        self.labels = labels;
        self
    }

    /// Add properties to the node
    pub fn with_properties(mut self, properties: Vec<(String, Expr)>) -> Self {
        self.properties = Some(properties);
        self
    }

    /// Add WHERE clause to the node
    pub fn with_where(mut self, where_clause: Expr) -> Self {
        self.where_clause = Some(where_clause);
        self
    }
}

/// Relationship pattern: -[variable:TYPE {properties}]->
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelPattern {
    /// Optional variable binding
    pub variable: Option<String>,
    /// Relationship types (can have multiple with |)
    pub types: Vec<String>,
    /// Property constraints
    pub properties: Option<Vec<(String, Expr)>>,
    /// Relationship direction
    pub direction: Direction,
    /// Variable length range (for path queries)
    pub range: Option<Range>,
    /// Inline WHERE clause
    pub where_clause: Option<Expr>,
}

impl RelPattern {
    /// Create a directed relationship with no constraints
    pub fn directed(direction: Direction) -> Self {
        Self {
            variable: None,
            types: Vec::new(),
            properties: None,
            direction,
            range: None,
            where_clause: None,
        }
    }

    /// Add variable to relationship
    pub fn with_variable(mut self, variable: impl Into<String>) -> Self {
        self.variable = Some(variable.into());
        self
    }

    /// Add types to relationship
    pub fn with_types(mut self, types: Vec<String>) -> Self {
        self.types = types;
        self
    }

    /// Add properties to relationship
    pub fn with_properties(mut self, properties: Vec<(String, Expr)>) -> Self {
        self.properties = Some(properties);
        self
    }

    /// Add range for variable-length paths
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = Some(range);
        self
    }
}

/// Relationship direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Direction {
    /// Left arrow: <-
    Left,
    /// Right arrow: ->
    Right,
    /// Both arrows: <->
    Both,
    /// No arrows: -
    None,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Direction::Left => write!(f, "<-"),
            Direction::Right => write!(f, "->"),
            Direction::Both => write!(f, "<->"),
            Direction::None => write!(f, "-"),
        }
    }
}

/// Variable-length path range
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Range {
    pub min: Option<u32>,
    pub max: Option<u32>,
}

impl Range {
    /// Create a range with both bounds
    pub fn bounded(min: u32, max: u32) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
        }
    }

    /// Create a range with only minimum
    pub fn min(min: u32) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Create a range with only maximum
    pub fn max(max: u32) -> Self {
        Self {
            min: None,
            max: Some(max),
        }
    }

    /// Create an unbounded range (*)
    pub fn unbounded() -> Self {
        Self {
            min: None,
            max: None,
        }
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (&self.min, &self.max) {
            (None, None) => write!(f, "*"),
            (Some(min), None) => write!(f, "*{}..", min),
            (None, Some(max)) => write!(f, "*..{}", max),
            (Some(min), Some(max)) => write!(f, "*{}..{}", min, max),
        }
    }
}
