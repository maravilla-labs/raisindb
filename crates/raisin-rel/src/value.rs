//! Runtime value types for expression evaluation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Runtime value during expression evaluation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    /// Create a null value
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean value
    pub fn boolean(b: bool) -> Self {
        Self::Boolean(b)
    }

    /// Create an integer value
    pub fn integer(n: i64) -> Self {
        Self::Integer(n)
    }

    /// Create a float value
    pub fn float(n: f64) -> Self {
        Self::Float(n)
    }

    /// Create a string value
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    /// Create an array value
    pub fn array(items: Vec<Value>) -> Self {
        Self::Array(items)
    }

    /// Create an object value
    pub fn object(fields: HashMap<String, Value>) -> Self {
        Self::Object(fields)
    }

    /// Convert from serde_json::Value
    pub fn from_json(json: serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Boolean(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    Self::Float(f)
                } else {
                    Self::Null
                }
            }
            serde_json::Value::String(s) => Self::String(s),
            serde_json::Value::Array(arr) => {
                Self::Array(arr.into_iter().map(Self::from_json).collect())
            }
            serde_json::Value::Object(obj) => Self::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, Self::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// Convert to serde_json::Value
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Boolean(b) => serde_json::Value::Bool(b),
            Self::Integer(i) => serde_json::Value::Number(i.into()),
            Self::Float(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::String(s) => serde_json::Value::String(s),
            Self::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(|v| v.to_json()).collect())
            }
            Self::Object(obj) => {
                serde_json::Value::Object(obj.into_iter().map(|(k, v)| (k, v.to_json())).collect())
            }
        }
    }

    /// Check if value is null
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if value is truthy (for boolean context)
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Boolean(b) => *b,
            Self::Integer(n) => *n != 0,
            Self::Float(n) => *n != 0.0,
            Self::String(s) => !s.is_empty(),
            Self::Array(arr) => !arr.is_empty(),
            Self::Object(obj) => !obj.is_empty(),
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            Self::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        }
    }

    /// Try to get as float (including integers)
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Integer(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Try to get as number (f64)
    pub fn as_number(&self) -> Option<f64> {
        self.as_float()
    }

    /// Try to get as string reference
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as array reference
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Try to get as object reference
    pub fn as_object(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get a property from an object
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(obj) => obj.get(key),
            _ => None,
        }
    }

    /// Get an element by index from an array
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Self::Array(arr) => arr.get(index),
            _ => None,
        }
    }

    /// Get the type name for error messages
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

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Boolean(b) => write!(f, "{}", b),
            Self::Integer(n) => write!(f, "{}", n),
            Self::Float(n) => write!(f, "{}", n),
            Self::String(s) => write!(f, "\"{}\"", s),
            Self::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Self::Object(obj) => {
                write!(f, "{{")?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::Null
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Boolean(b)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Self::Integer(n)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Self::Integer(n as i64)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Self::Float(n)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Self::Array(v.into_iter().map(Into::into).collect())
    }
}

impl From<HashMap<String, Value>> for Value {
    fn from(m: HashMap<String, Value>) -> Self {
        Self::Object(m)
    }
}

impl From<serde_json::Value> for Value {
    fn from(json: serde_json::Value) -> Self {
        Self::from_json(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_json() {
        let json = serde_json::json!({
            "name": "test",
            "count": 42,
            "active": true,
            "tags": ["a", "b"],
            "meta": null
        });

        let value = Value::from_json(json);
        assert!(matches!(value, Value::Object(_)));

        if let Value::Object(obj) = value {
            assert_eq!(obj.get("name"), Some(&Value::String("test".to_string())));
            assert_eq!(obj.get("count"), Some(&Value::Integer(42)));
            assert_eq!(obj.get("active"), Some(&Value::Boolean(true)));
            assert_eq!(obj.get("meta"), Some(&Value::Null));
        }
    }

    #[test]
    fn test_to_json() {
        let mut obj = HashMap::new();
        obj.insert("name".to_string(), Value::String("test".to_string()));
        obj.insert("count".to_string(), Value::Integer(42));

        let value = Value::Object(obj);
        let json = value.to_json();

        assert_eq!(json["name"], "test");
        assert_eq!(json["count"], 42);
    }

    #[test]
    fn test_is_truthy() {
        assert!(!Value::Null.is_truthy());
        assert!(!Value::Boolean(false).is_truthy());
        assert!(Value::Boolean(true).is_truthy());
        assert!(!Value::Integer(0).is_truthy());
        assert!(Value::Integer(1).is_truthy());
        assert!(!Value::String("".to_string()).is_truthy());
        assert!(Value::String("hello".to_string()).is_truthy());
        assert!(!Value::Array(vec![]).is_truthy());
        assert!(Value::Array(vec![Value::Integer(1)]).is_truthy());
    }

    #[test]
    fn test_type_coercion() {
        assert_eq!(Value::Integer(42).as_float(), Some(42.0));
        assert_eq!(Value::Float(42.0).as_integer(), Some(42));
        assert_eq!(Value::Float(42.5).as_integer(), None);
    }
}
