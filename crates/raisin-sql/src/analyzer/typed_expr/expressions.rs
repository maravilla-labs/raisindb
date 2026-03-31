//! Core typed expression types
//!
//! Contains the main `TypedExpr`, `Expr`, and `Literal` types used throughout
//! the semantic analysis pipeline.

use super::operators::BinaryOperator;
use super::operators::UnaryOperator;
use super::window::{WindowFrame, WindowFunction};
use crate::analyzer::functions::FunctionSignature;
use crate::analyzer::types::DataType;

/// Typed expression (output of semantic analysis)
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub expr: Expr,
    pub data_type: DataType,
}

impl TypedExpr {
    /// Create a new typed expression
    pub fn new(expr: Expr, data_type: DataType) -> Self {
        Self { expr, data_type }
    }

    /// Create a literal expression
    pub fn literal(literal: Literal) -> Self {
        let data_type = literal.data_type();
        Self {
            expr: Expr::Literal(literal),
            data_type,
        }
    }

    /// Create a column reference expression
    pub fn column(table: String, column: String, data_type: DataType) -> Self {
        Self {
            expr: Expr::Column { table, column },
            data_type,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    Literal(Literal),

    // Column reference (qualified)
    Column {
        table: String,
        column: String,
    },

    // Function call
    Function {
        name: String,
        args: Vec<TypedExpr>,
        signature: FunctionSignature,
        /// Optional FILTER clause for aggregate functions
        filter: Option<Box<TypedExpr>>,
    },

    // Operators
    BinaryOp {
        left: Box<TypedExpr>,
        op: BinaryOperator,
        right: Box<TypedExpr>,
    },

    UnaryOp {
        op: UnaryOperator,
        expr: Box<TypedExpr>,
    },

    // Special
    Cast {
        expr: Box<TypedExpr>,
        target_type: DataType,
    },
    IsNull {
        expr: Box<TypedExpr>,
    },
    IsNotNull {
        expr: Box<TypedExpr>,
    },
    Between {
        expr: Box<TypedExpr>,
        low: Box<TypedExpr>,
        high: Box<TypedExpr>,
    },
    InList {
        expr: Box<TypedExpr>,
        list: Vec<TypedExpr>,
        negated: bool,
    },

    /// IN subquery: expr IN (SELECT col FROM ...)
    /// The subquery must return exactly one column with a type compatible with expr.
    /// During logical plan building, this is transformed into a SemiJoin.
    InSubquery {
        /// The expression to check for membership
        expr: Box<TypedExpr>,
        /// The analyzed subquery (must have exactly one projection column)
        subquery: Box<crate::analyzer::semantic::AnalyzedQuery>,
        /// The data type of the subquery's single column (for type checking)
        subquery_type: crate::analyzer::types::DataType,
        /// Whether this is NOT IN (anti-semi-join)
        negated: bool,
    },

    /// LIKE pattern matching: name LIKE 'test%'
    Like {
        expr: Box<TypedExpr>,
        pattern: Box<TypedExpr>,
        negated: bool,
    },

    /// ILIKE pattern matching (case-insensitive): name ILIKE 'test%'
    ILike {
        expr: Box<TypedExpr>,
        pattern: Box<TypedExpr>,
        negated: bool,
    },

    /// JSON object extraction: properties -> 'key'
    /// Returns JSONB (use JsonExtractText for text extraction with ->>)
    JsonExtract {
        object: Box<TypedExpr>,
        key: Box<TypedExpr>,
    },

    /// JSON text extraction: properties ->> 'key'
    JsonExtractText {
        object: Box<TypedExpr>,
        key: Box<TypedExpr>,
    },

    /// JSON containment: properties @> '{"key": "value"}'
    JsonContains {
        object: Box<TypedExpr>,
        pattern: Box<TypedExpr>,
    },

    /// JSON key existence: properties ? 'key'
    /// Returns BOOLEAN indicating if the key exists in the JSON object
    JsonKeyExists {
        object: Box<TypedExpr>,
        key: Box<TypedExpr>,
    },

    /// JSON any key exists: properties ?| ARRAY['key1', 'key2']
    /// Returns BOOLEAN indicating if ANY of the keys exist in the JSON object
    JsonAnyKeyExists {
        object: Box<TypedExpr>,
        keys: Box<TypedExpr>,
    },

    /// JSON all keys exist: properties ?& ARRAY['key1', 'key2']
    /// Returns BOOLEAN indicating if ALL of the keys exist in the JSON object
    JsonAllKeyExists {
        object: Box<TypedExpr>,
        keys: Box<TypedExpr>,
    },

    /// JSON extract at path: properties #> ARRAY['metadata', 'author']
    /// Returns JSONB? value at the specified path
    JsonExtractPath {
        object: Box<TypedExpr>,
        path: Box<TypedExpr>,
    },

    /// JSON extract at path as text: properties #>> ARRAY['metadata', 'author']
    /// Returns TEXT? value at the specified path
    JsonExtractPathText {
        object: Box<TypedExpr>,
        path: Box<TypedExpr>,
    },

    /// JSON remove key/element: properties - 'key' or properties - 1 or properties - '["key1", "key2"]'
    /// Removes a key from object, element at index from array, or multiple keys from object
    /// Returns JSONB
    JsonRemove {
        object: Box<TypedExpr>,
        key: Box<TypedExpr>,
    },

    /// JSON remove at path: properties #- ARRAY['metadata', 'author']
    /// Removes the value at the specified path from a JSONB value
    /// Returns JSONB
    JsonRemoveAtPath {
        object: Box<TypedExpr>,
        path: Box<TypedExpr>,
    },

    /// JSON path match: properties @@ '$.tags[*] ? (@ == "rust")'
    /// Tests if a JSONPath predicate matches the JSON value
    /// Returns BOOLEAN
    JsonPathMatch {
        object: Box<TypedExpr>,
        path: Box<TypedExpr>,
    },

    /// JSON path exists: properties @? '$.metadata.author'
    /// Tests if a JSONPath expression has any matches
    /// Returns BOOLEAN
    JsonPathExists {
        object: Box<TypedExpr>,
        path: Box<TypedExpr>,
    },

    /// Window function with OVER clause
    /// Example: ROW_NUMBER() OVER (PARTITION BY parent ORDER BY version DESC)
    Window {
        function: WindowFunction,
        partition_by: Vec<TypedExpr>,
        order_by: Vec<(TypedExpr, bool)>, // (expr, is_desc)
        frame: Option<WindowFrame>,
    },

    /// CASE expression for conditional logic
    ///
    /// Supports both simple and searched CASE expressions:
    /// - Searched CASE: `CASE WHEN x > 10 THEN 'high' WHEN x > 5 THEN 'medium' ELSE 'low' END`
    /// - Simple CASE: Converted to searched form by parser
    ///
    /// # Semantics
    /// - Evaluates conditions in order until one returns TRUE
    /// - Returns the corresponding result expression
    /// - If no condition matches, returns ELSE expression (or NULL if no ELSE)
    /// - All result expressions must have compatible types
    /// - The overall return type is the common type of all branches (including ELSE)
    Case {
        /// List of (condition, result) pairs - WHEN clauses
        /// Each condition must evaluate to BOOLEAN
        /// Evaluated in order until one is TRUE
        conditions: Vec<(TypedExpr, TypedExpr)>,
        /// Optional ELSE expression
        /// If None, defaults to NULL
        else_expr: Option<Box<TypedExpr>>,
    },
}

/// Literal values
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    Int(i32),
    BigInt(i64),
    Double(f64),
    Text(String),
    Uuid(String),
    Path(String),
    JsonB(serde_json::Value),
    /// Vector embedding for similarity search
    Vector(Vec<f32>),
    /// GeoJSON geometry (Point, LineString, Polygon, etc.)
    /// Used for geospatial queries with PostGIS-compatible ST_* functions
    Geometry(serde_json::Value),
    /// Timestamp with timezone (UTC)
    Timestamp(chrono::DateTime<chrono::Utc>),
    /// Time interval/duration
    Interval(chrono::Duration),
    /// Parameter placeholder ($1, $2, etc.)
    Parameter(String),
}

impl Literal {
    /// Get the data type of this literal
    pub fn data_type(&self) -> DataType {
        match self {
            Literal::Null => DataType::Unknown, // Null can be any nullable type
            Literal::Boolean(_) => DataType::Boolean,
            Literal::Int(_) => DataType::Int,
            Literal::BigInt(_) => DataType::BigInt,
            Literal::Double(_) => DataType::Double,
            Literal::Text(_) => DataType::Text,
            Literal::Uuid(_) => DataType::Uuid,
            Literal::Path(_) => DataType::Path,
            Literal::JsonB(_) => DataType::JsonB,
            Literal::Vector(v) => DataType::Vector(v.len()),
            Literal::Geometry(_) => DataType::Geometry,
            Literal::Timestamp(_) => DataType::TimestampTz,
            Literal::Interval(_) => DataType::Interval,
            Literal::Parameter(_) => DataType::Unknown, // Parameters have unknown type until bound
        }
    }
}
