// SPDX-License-Identifier: BSL-1.1

//! Expression AST nodes

use serde::Serialize;
use std::fmt;

/// Expression node
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Expr {
    /// Literal value
    Literal(Literal),

    /// Parameter reference ($param)
    Parameter(String),

    /// Variable reference
    Variable(String),

    /// Property access (expr.property)
    Property { expr: Box<Expr>, property: String },

    /// Binary operation
    BinaryOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },

    /// Unary operation
    UnaryOp { op: UnOp, expr: Box<Expr> },

    /// Function call
    FunctionCall {
        name: String,
        distinct: bool,
        args: Vec<Expr>,
    },

    /// List construction [expr, ...]
    List(Vec<Expr>),

    /// Map construction {key: value, ...}
    Map(Vec<(String, Expr)>),

    /// CASE expression
    Case {
        operand: Option<Box<Expr>>,
        when_branches: Vec<(Expr, Expr)>,
        else_branch: Option<Box<Expr>>,
    },
}

impl Expr {
    /// Create a variable expression
    pub fn variable(name: impl Into<String>) -> Self {
        Self::Variable(name.into())
    }

    /// Create a property access expression
    pub fn property(expr: Expr, property: impl Into<String>) -> Self {
        Self::Property {
            expr: Box::new(expr),
            property: property.into(),
        }
    }

    /// Create a binary operation
    pub fn binary(left: Expr, op: BinOp, right: Expr) -> Self {
        Self::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    /// Create a unary operation
    pub fn unary(op: UnOp, expr: Expr) -> Self {
        Self::UnaryOp {
            expr: Box::new(expr),
            op,
        }
    }

    /// Create a function call
    pub fn function(name: impl Into<String>, args: Vec<Expr>) -> Self {
        Self::FunctionCall {
            name: name.into(),
            distinct: false,
            args,
        }
    }
}

/// Literal value
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Literal {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Literal::Null => write!(f, "null"),
            Literal::Boolean(b) => write!(f, "{}", b),
            Literal::Integer(i) => write!(f, "{}", i),
            Literal::Float(fl) => write!(f, "{}", fl),
            Literal::String(s) => write!(f, "\"{}\"", s),
        }
    }
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BinOp {
    // Logical
    Or,
    Xor,
    And,

    // Comparison
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,

    // String operations
    StartsWith,
    EndsWith,
    Contains,
    RegexMatch,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,

    // Collection
    In,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BinOp::Or => write!(f, "OR"),
            BinOp::Xor => write!(f, "XOR"),
            BinOp::And => write!(f, "AND"),
            BinOp::Eq => write!(f, "="),
            BinOp::Neq => write!(f, "<>"),
            BinOp::Lt => write!(f, "<"),
            BinOp::Lte => write!(f, "<="),
            BinOp::Gt => write!(f, ">"),
            BinOp::Gte => write!(f, ">="),
            BinOp::StartsWith => write!(f, "STARTS WITH"),
            BinOp::EndsWith => write!(f, "ENDS WITH"),
            BinOp::Contains => write!(f, "CONTAINS"),
            BinOp::RegexMatch => write!(f, "=~"),
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "%"),
            BinOp::Pow => write!(f, "^"),
            BinOp::In => write!(f, "IN"),
        }
    }
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UnOp {
    Not,
    Plus,
    Minus,
    IsNull,
    IsNotNull,
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UnOp::Not => write!(f, "NOT"),
            UnOp::Plus => write!(f, "+"),
            UnOp::Minus => write!(f, "-"),
            UnOp::IsNull => write!(f, "IS NULL"),
            UnOp::IsNotNull => write!(f, "IS NOT NULL"),
        }
    }
}
