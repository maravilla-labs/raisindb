//! Wrapper and error-handling clause types for JSON_QUERY
//!
//! Implements the SQL:2016 clause behaviors:
//! - `WrapperClause` - controls array wrapping of results
//! - `OnEmptyBehavior` - controls behavior when a path has no matches
//! - `OnErrorBehavior` - controls behavior when JSONPath evaluation fails

use raisin_error::Error;
use raisin_sql::analyzer::Literal;

/// Wrapper clause behavior for JSON_QUERY
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum WrapperClause {
    /// WITHOUT WRAPPER (default): Returns NULL if multiple matches
    Without,
    /// WITH WRAPPER: Always wraps result in an array
    With,
    /// WITH CONDITIONAL WRAPPER: Wraps only if multiple matches
    Conditional,
}

impl WrapperClause {
    /// Parse wrapper clause from TEXT parameter
    pub(crate) fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_uppercase().as_str() {
            "WITHOUT WRAPPER" | "WITHOUT_WRAPPER" => Ok(WrapperClause::Without),
            "WITH WRAPPER" | "WITH_WRAPPER" => Ok(WrapperClause::With),
            "WITH CONDITIONAL WRAPPER" | "WITH_CONDITIONAL_WRAPPER" | "CONDITIONAL" => {
                Ok(WrapperClause::Conditional)
            }
            _ => Err(Error::Validation(format!(
                "Invalid wrapper clause '{}'. Expected: 'WITH WRAPPER', 'WITHOUT WRAPPER', or 'WITH CONDITIONAL WRAPPER'",
                s
            ))),
        }
    }
}

/// ON EMPTY behavior for JSON_QUERY
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OnEmptyBehavior {
    /// NULL ON EMPTY (default): Return NULL when path doesn't exist or result is empty
    Null,
    /// ERROR ON EMPTY: Raise an error when path doesn't exist or result is empty
    Error,
    /// EMPTY ARRAY ON EMPTY: Return empty array [] when path doesn't exist or result is empty
    EmptyArray,
    /// EMPTY OBJECT ON EMPTY: Return empty object {} when path doesn't exist or result is empty
    EmptyObject,
}

impl OnEmptyBehavior {
    /// Parse ON EMPTY behavior from TEXT parameter
    pub(crate) fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_uppercase().as_str() {
            "NULL" | "NULL ON EMPTY" => Ok(OnEmptyBehavior::Null),
            "ERROR" | "ERROR ON EMPTY" => Ok(OnEmptyBehavior::Error),
            "EMPTY ARRAY" | "EMPTY_ARRAY" | "EMPTY ARRAY ON EMPTY" => {
                Ok(OnEmptyBehavior::EmptyArray)
            }
            "EMPTY OBJECT" | "EMPTY_OBJECT" | "EMPTY OBJECT ON EMPTY" => {
                Ok(OnEmptyBehavior::EmptyObject)
            }
            _ => Err(Error::Validation(format!(
                "Invalid ON EMPTY behavior '{}'. Expected: 'NULL', 'ERROR', 'EMPTY ARRAY', or 'EMPTY OBJECT'",
                s
            ))),
        }
    }

    /// Produce the result value dictated by this behavior
    pub(crate) fn apply(self) -> Result<Literal, Error> {
        match self {
            OnEmptyBehavior::Null => Ok(Literal::Null),
            OnEmptyBehavior::Error => Err(Error::Validation(
                "JSON_QUERY: path does not exist or result is empty".to_string(),
            )),
            OnEmptyBehavior::EmptyArray => Ok(Literal::JsonB(serde_json::Value::Array(vec![]))),
            OnEmptyBehavior::EmptyObject => Ok(Literal::JsonB(serde_json::Value::Object(
                serde_json::Map::new(),
            ))),
        }
    }
}

/// ON ERROR behavior for JSON_QUERY
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OnErrorBehavior {
    /// NULL ON ERROR (default): Return NULL when an error occurs
    Null,
    /// ERROR ON ERROR: Propagate the error when it occurs
    Error,
    /// EMPTY ARRAY ON ERROR: Return empty array [] when an error occurs
    EmptyArray,
    /// EMPTY OBJECT ON ERROR: Return empty object {} when an error occurs
    EmptyObject,
}

impl OnErrorBehavior {
    /// Parse ON ERROR behavior from TEXT parameter
    pub(crate) fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_uppercase().as_str() {
            "NULL" | "NULL ON ERROR" => Ok(OnErrorBehavior::Null),
            "ERROR" | "ERROR ON ERROR" => Ok(OnErrorBehavior::Error),
            "EMPTY ARRAY" | "EMPTY_ARRAY" | "EMPTY ARRAY ON ERROR" => {
                Ok(OnErrorBehavior::EmptyArray)
            }
            "EMPTY OBJECT" | "EMPTY_OBJECT" | "EMPTY OBJECT ON ERROR" => {
                Ok(OnErrorBehavior::EmptyObject)
            }
            _ => Err(Error::Validation(format!(
                "Invalid ON ERROR behavior '{}'. Expected: 'NULL', 'ERROR', 'EMPTY ARRAY', or 'EMPTY OBJECT'",
                s
            ))),
        }
    }

    /// Produce the result value dictated by this behavior
    pub(crate) fn apply(self, error_msg: String) -> Result<Literal, Error> {
        match self {
            OnErrorBehavior::Null => Ok(Literal::Null),
            OnErrorBehavior::Error => Err(Error::Validation(error_msg)),
            OnErrorBehavior::EmptyArray => Ok(Literal::JsonB(serde_json::Value::Array(vec![]))),
            OnErrorBehavior::EmptyObject => Ok(Literal::JsonB(serde_json::Value::Object(
                serde_json::Map::new(),
            ))),
        }
    }
}
