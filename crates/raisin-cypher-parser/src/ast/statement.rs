// SPDX-License-Identifier: BSL-1.1

//! Statement AST nodes

use super::expr::Expr;
use super::pattern::GraphPattern;
use serde::Serialize;

/// Top-level statement
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Statement {
    /// Query statement (MATCH, CREATE, RETURN, etc.)
    Query(Query),
}

/// A complete Cypher query
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Query {
    /// Query clauses in order
    pub clauses: Vec<Clause>,
}

impl Query {
    /// Create a new query from clauses
    pub fn new(clauses: Vec<Clause>) -> Self {
        Self { clauses }
    }

    /// Create a query with a single clause
    pub fn single(clause: Clause) -> Self {
        Self {
            clauses: vec![clause],
        }
    }
}

/// Query clause
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Clause {
    /// MATCH clause
    Match {
        optional: bool,
        pattern: GraphPattern,
    },

    /// CREATE clause
    Create { pattern: GraphPattern },

    /// MERGE clause
    Merge { pattern: GraphPattern },

    /// WHERE clause (typically follows MATCH or WITH)
    Where { condition: Expr },

    /// RETURN clause
    Return {
        distinct: bool,
        items: Vec<ReturnItem>,
        order_by: Vec<OrderBy>,
        skip: Option<Expr>,
        limit: Option<Expr>,
    },

    /// WITH clause (projection with pass-through)
    With {
        distinct: bool,
        items: Vec<ReturnItem>,
        order_by: Vec<OrderBy>,
        skip: Option<Expr>,
        limit: Option<Expr>,
        where_clause: Option<Expr>,
    },

    /// SET clause
    Set { items: Vec<SetItem> },

    /// DELETE clause
    Delete { detach: bool, items: Vec<Expr> },

    /// REMOVE clause
    Remove { items: Vec<RemoveItem> },

    /// UNWIND clause
    Unwind { expr: Expr, alias: String },
}

impl Clause {
    /// Create a MATCH clause
    pub fn match_pattern(pattern: GraphPattern) -> Self {
        Self::Match {
            optional: false,
            pattern,
        }
    }

    /// Create an OPTIONAL MATCH clause
    pub fn optional_match(pattern: GraphPattern) -> Self {
        Self::Match {
            optional: true,
            pattern,
        }
    }

    /// Create a CREATE clause
    pub fn create(pattern: GraphPattern) -> Self {
        Self::Create { pattern }
    }

    /// Create a WHERE clause
    pub fn where_clause(condition: Expr) -> Self {
        Self::Where { condition }
    }

    /// Create a RETURN clause
    pub fn return_items(items: Vec<ReturnItem>) -> Self {
        Self::Return {
            distinct: false,
            items,
            order_by: Vec::new(),
            skip: None,
            limit: None,
        }
    }
}

/// Return item (expression with optional alias)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReturnItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

impl ReturnItem {
    /// Create a return item without alias
    pub fn new(expr: Expr) -> Self {
        Self { expr, alias: None }
    }

    /// Create a return item with alias
    pub fn with_alias(expr: Expr, alias: impl Into<String>) -> Self {
        Self {
            expr,
            alias: Some(alias.into()),
        }
    }
}

/// ORDER BY specification
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrderBy {
    pub expr: Expr,
    pub order: Order,
}

impl OrderBy {
    /// Create an ORDER BY ASC
    pub fn asc(expr: Expr) -> Self {
        Self {
            expr,
            order: Order::Asc,
        }
    }

    /// Create an ORDER BY DESC
    pub fn desc(expr: Expr) -> Self {
        Self {
            expr,
            order: Order::Desc,
        }
    }
}

/// Sort order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Order {
    Asc,
    Desc,
}

/// SET clause item
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum SetItem {
    /// Set property: var.prop = value
    Property {
        variable: String,
        property: String,
        value: Expr,
    },
    /// Set variable: var = value (replace all properties)
    Variable { variable: String, value: Expr },
    /// Add properties: var += {props}
    AddProperties { variable: String, properties: Expr },
    /// Set labels: var:Label
    Labels {
        variable: String,
        labels: Vec<String>,
    },
}

/// REMOVE clause item
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum RemoveItem {
    /// Remove property: var.prop
    Property { variable: String, property: String },
    /// Remove labels: var:Label
    Labels {
        variable: String,
        labels: Vec<String>,
    },
}
