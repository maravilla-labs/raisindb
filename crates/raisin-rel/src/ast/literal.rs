//! Literal value AST nodes

use serde::{Deserialize, Serialize};

/// Literal values in expressions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    /// Null value
    Null,
    /// Boolean value: true or false
    Boolean(bool),
    /// Integer value: 42, -10
    Integer(i64),
    /// Floating point value: 3.14, -0.5
    Float(f64),
    /// String value: 'hello' or "world"
    String(String),
    /// Array literal: [1, 2, 3]
    Array(Vec<Literal>),
    /// Object literal: {key: 'value', num: 42}
    Object(Vec<(String, Literal)>),
}

impl Literal {
    /// Create a null literal
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean literal
    pub fn boolean(b: bool) -> Self {
        Self::Boolean(b)
    }

    /// Create an integer literal
    pub fn integer(n: i64) -> Self {
        Self::Integer(n)
    }

    /// Create a float literal
    pub fn float(n: f64) -> Self {
        Self::Float(n)
    }

    /// Create a string literal
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    /// Create an array literal
    pub fn array(items: Vec<Literal>) -> Self {
        Self::Array(items)
    }

    /// Create an object literal
    pub fn object(fields: Vec<(String, Literal)>) -> Self {
        Self::Object(fields)
    }

    /// Check if this is a null literal
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if this is a boolean literal
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Boolean(_))
    }

    /// Check if this is a numeric literal (integer or float)
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::Float(_))
    }

    /// Check if this is a string literal
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    /// Check if this is an array literal
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    /// Check if this is an object literal
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// Get the type name for display
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }
}

impl std::fmt::Display for Literal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Boolean(b) => write!(f, "{}", b),
            Self::Integer(n) => write!(f, "{}", n),
            Self::Float(n) => write!(f, "{}", n),
            Self::String(s) => write!(f, "'{}'", s),
            Self::Array(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Self::Object(fields) => {
                write!(f, "{{")?;
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", key, value)?;
                }
                write!(f, "}}")
            }
        }
    }
}
