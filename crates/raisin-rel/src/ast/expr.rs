//! Expression AST nodes

use super::literal::Literal;
use serde::{Deserialize, Serialize};

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    // Comparison operators
    /// Equals: ==
    Eq,
    /// Not equals: !=
    Neq,
    /// Less than: <
    Lt,
    /// Greater than: >
    Gt,
    /// Less than or equal: <=
    Lte,
    /// Greater than or equal: >=
    Gte,

    // Logical operators
    /// Logical AND: &&
    And,
    /// Logical OR: ||
    Or,

    // Arithmetic operators
    /// Addition: +
    Add,
    /// Subtraction: -
    Sub,
    /// Multiplication: *
    Mul,
    /// Division: /
    Div,
    /// Modulo: %
    Mod,
}

impl BinOp {
    /// Get the operator symbol for display
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Neq => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Lte => "<=",
            Self::Gte => ">=",
            Self::And => "&&",
            Self::Or => "||",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
        }
    }

    /// Check if this is a comparison operator
    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Neq | Self::Lt | Self::Gt | Self::Lte | Self::Gte
        )
    }

    /// Check if this is a logical operator
    pub fn is_logical(&self) -> bool {
        matches!(self, Self::And | Self::Or)
    }

    /// Check if this is an arithmetic operator
    pub fn is_arithmetic(&self) -> bool {
        matches!(
            self,
            Self::Add | Self::Sub | Self::Mul | Self::Div | Self::Mod
        )
    }
}

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    /// Logical NOT: !
    Not,
    /// Unary minus: -
    Neg,
}

impl UnOp {
    /// Get the operator symbol for display
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Not => "!",
            Self::Neg => "-",
        }
    }
}

impl std::fmt::Display for UnOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

/// Direction for graph relationship traversal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelDirection {
    /// Follow relationships outgoing from source node
    Outgoing,
    /// Follow relationships incoming to source node
    Incoming,
    /// Follow relationships in any direction
    Any,
}

impl Default for RelDirection {
    fn default() -> Self {
        Self::Any
    }
}

impl std::fmt::Display for RelDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outgoing => write!(f, "OUTGOING"),
            Self::Incoming => write!(f, "INCOMING"),
            Self::Any => write!(f, "ANY"),
        }
    }
}

/// Expression AST node
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// Literal value: 42, "hello", true, null
    Literal(Literal),

    /// Variable reference: input, context
    Variable(String),

    /// Property access: input.value, context.user.name
    PropertyAccess { object: Box<Expr>, property: String },

    /// Index access: input.tags[0], data["key"]
    IndexAccess { object: Box<Expr>, index: Box<Expr> },

    /// Binary operation: a == b, x > 10
    BinaryOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },

    /// Unary operation: !condition, -value
    UnaryOp { op: UnOp, expr: Box<Expr> },

    /// Method call on an expression: expr.method(args)
    /// Example: input.text.contains('hello'), input.name.toLowerCase()
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },

    /// Grouping: (expr)
    Grouped(Box<Expr>),

    /// Graph relationship check: source RELATES target VIA 'TYPE'
    /// Example: node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2
    Relates {
        source: Box<Expr>,
        target: Box<Expr>,
        relation_types: Vec<String>,
        min_depth: u32,
        max_depth: u32,
        direction: RelDirection,
    },
}

impl Expr {
    /// Create a literal expression
    pub fn literal(lit: Literal) -> Self {
        Self::Literal(lit)
    }

    /// Create a variable expression
    pub fn variable(name: impl Into<String>) -> Self {
        Self::Variable(name.into())
    }

    /// Create a property access expression
    pub fn property_access(object: Expr, property: impl Into<String>) -> Self {
        Self::PropertyAccess {
            object: Box::new(object),
            property: property.into(),
        }
    }

    /// Create an index access expression
    pub fn index_access(object: Expr, index: Expr) -> Self {
        Self::IndexAccess {
            object: Box::new(object),
            index: Box::new(index),
        }
    }

    /// Create a binary operation expression
    pub fn binary(left: Expr, op: BinOp, right: Expr) -> Self {
        Self::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    /// Create a unary operation expression
    pub fn unary(op: UnOp, expr: Expr) -> Self {
        Self::UnaryOp {
            op,
            expr: Box::new(expr),
        }
    }

    /// Create a method call expression
    pub fn method_call(object: Expr, method: impl Into<String>, args: Vec<Expr>) -> Self {
        Self::MethodCall {
            object: Box::new(object),
            method: method.into(),
            args,
        }
    }

    /// Create a grouped expression
    pub fn grouped(expr: Expr) -> Self {
        Self::Grouped(Box::new(expr))
    }

    /// Check if this is a literal expression
    pub fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// Check if this is a variable expression
    pub fn is_variable(&self) -> bool {
        matches!(self, Self::Variable(_))
    }

    /// Check if this is a binary operation
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::BinaryOp { .. })
    }

    /// Check if this is a unary operation
    pub fn is_unary(&self) -> bool {
        matches!(self, Self::UnaryOp { .. })
    }

    /// Check if this is a method call
    pub fn is_method_call(&self) -> bool {
        matches!(self, Self::MethodCall { .. })
    }

    /// Check if this is a relates expression
    pub fn is_relates(&self) -> bool {
        matches!(self, Self::Relates { .. })
    }

    /// Create a relates expression
    pub fn relates(
        source: Expr,
        target: Expr,
        relation_types: Vec<String>,
        min_depth: u32,
        max_depth: u32,
        direction: RelDirection,
    ) -> Self {
        Self::Relates {
            source: Box::new(source),
            target: Box::new(target),
            relation_types,
            min_depth,
            max_depth,
            direction,
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal(lit) => write!(f, "{}", lit),
            Self::Variable(name) => write!(f, "{}", name),
            Self::PropertyAccess { object, property } => write!(f, "{}.{}", object, property),
            Self::IndexAccess { object, index } => write!(f, "{}[{}]", object, index),
            Self::BinaryOp { left, op, right } => write!(f, "{} {} {}", left, op, right),
            Self::UnaryOp { op, expr } => write!(f, "{}{}", op, expr),
            Self::MethodCall {
                object,
                method,
                args,
            } => {
                write!(f, "{}.{}(", object, method)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Self::Grouped(expr) => write!(f, "({})", expr),
            Self::Relates {
                source,
                target,
                relation_types,
                min_depth,
                max_depth,
                direction,
            } => {
                write!(f, "{} RELATES {} VIA ", source, target)?;
                if relation_types.len() == 1 {
                    write!(f, "'{}'", relation_types[0])?;
                } else {
                    write!(f, "[")?;
                    for (i, rt) in relation_types.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "'{}'", rt)?;
                    }
                    write!(f, "]")?;
                }

                // Only include DEPTH if not default (1..1)
                if *min_depth != 1 || *max_depth != 1 {
                    write!(f, " DEPTH {}..{}", min_depth, max_depth)?;
                }

                // Only include DIRECTION if not default (Any)
                if *direction != RelDirection::Any {
                    write!(f, " DIRECTION {}", direction)?;
                }

                Ok(())
            }
        }
    }
}
