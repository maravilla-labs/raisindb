//! PGQ expression types
//!
//! Defines the expression AST used within SQL/PGQ GRAPH_TABLE queries,
//! including property access, literals, operators, and special forms.

use serde::{Deserialize, Serialize};

use super::query::SourceSpan;

/// Expression types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// Property access: node.property or node.property.nested
    PropertyAccess {
        /// Variable name
        variable: String,
        /// Property path
        properties: Vec<String>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// Literal value
    Literal(Literal),

    /// Binary operation: a = b, a AND b, a + b
    BinaryOp {
        /// Left operand
        left: Box<Expr>,
        /// Operator
        op: BinaryOperator,
        /// Right operand
        right: Box<Expr>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// Unary operation: NOT a, -a
    UnaryOp {
        /// Operator
        op: UnaryOperator,
        /// Operand
        expr: Box<Expr>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// Function call: degree(n), shortestPath(a, b)
    FunctionCall {
        /// Function name
        name: String,
        /// Arguments
        args: Vec<Expr>,
        /// DISTINCT modifier (for aggregates)
        distinct: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// IS NULL
    IsNull {
        /// Expression to check
        expr: Box<Expr>,
        /// Negated (IS NOT NULL)
        negated: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// IN list: a IN (1, 2, 3)
    InList {
        /// Expression to check
        expr: Box<Expr>,
        /// List of values
        list: Vec<Expr>,
        /// Negated (NOT IN)
        negated: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// BETWEEN: a BETWEEN low AND high
    Between {
        /// Expression to check
        expr: Box<Expr>,
        /// Lower bound
        low: Box<Expr>,
        /// Upper bound
        high: Box<Expr>,
        /// Negated (NOT BETWEEN)
        negated: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// LIKE pattern: a LIKE '%pattern%'
    Like {
        /// Expression to check
        expr: Box<Expr>,
        /// Pattern
        pattern: String,
        /// Negated (NOT LIKE)
        negated: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// CASE expression
    Case {
        /// Operand (for simple CASE)
        operand: Option<Box<Expr>>,
        /// WHEN clauses
        when_clauses: Vec<(Expr, Expr)>,
        /// ELSE clause
        else_clause: Option<Box<Expr>>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// Wildcard: * or node.*
    Wildcard {
        /// Optional qualifier
        qualifier: Option<String>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// Parenthesized expression
    Nested(Box<Expr>),

    /// JSON key access using -> or ->> operators
    /// e.g., properties->>'name' or properties->'address'->>'city'
    JsonAccess {
        /// Base expression (usually PropertyAccess)
        expr: Box<Expr>,
        /// Key to access
        key: String,
        /// True for ->> (extract as text), false for -> (extract as JSON)
        as_text: bool,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },

    /// JSONPath-style variable access: $.variable.path.to.value
    /// This is sugar for variable.properties.path.to.value
    JsonPathAccess {
        /// Variable name (e.g., "friend" from $.friend.properties.email)
        variable: String,
        /// Path segments after the variable (e.g., ["properties", "email"])
        path: Vec<String>,
        /// Source location
        #[serde(default)]
        span: SourceSpan,
    },
}

/// Literal values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    /// String: 'hello'
    String(String),
    /// Integer: 42
    Integer(i64),
    /// Float: 3.14
    Float(f64),
    /// Boolean: true, false
    Boolean(bool),
    /// Null
    Null,
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOperator {
    // Comparison
    /// =
    Eq,
    /// <> or !=
    NotEq,
    /// <
    Lt,
    /// <=
    LtEq,
    /// >
    Gt,
    /// >=
    GtEq,

    // Logical
    /// AND
    And,
    /// OR
    Or,

    // Arithmetic
    /// +
    Plus,
    /// -
    Minus,
    /// *
    Multiply,
    /// /
    Divide,
    /// %
    Modulo,

    // String
    /// || (concatenation)
    Concat,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOperator {
    /// NOT
    Not,
    /// - (negation)
    Minus,
    /// + (positive, no-op)
    Plus,
}
