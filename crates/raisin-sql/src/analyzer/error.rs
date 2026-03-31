use thiserror::Error;

pub type Result<T> = std::result::Result<T, AnalysisError>;

#[derive(Error, Debug)]
pub enum AnalysisError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Empty statement")]
    EmptyStatement,

    #[error("Multiple statements not supported")]
    MultipleStatements,

    #[error("Unsupported statement: {0}")]
    UnsupportedStatement(String),

    #[error("Table not found: {0}")]
    TableNotFound(String),

    #[error("Column not found: {table}.{column}")]
    ColumnNotFound { table: String, column: String },

    #[error("Ambiguous column reference: {0}")]
    AmbiguousColumn(String),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("Function not found: {name}({args})")]
    FunctionNotFound { name: String, args: String },

    #[error("Invalid argument count for {function}: expected {expected}, got {actual}")]
    InvalidArgumentCount {
        function: String,
        expected: usize,
        actual: usize,
    },

    #[error("Cannot coerce type {from} to {to}")]
    InvalidCoercion { from: String, to: String },

    #[error("Invalid binary operation: {left} {op} {right}")]
    InvalidBinaryOp {
        left: String,
        op: String,
        right: String,
    },

    #[error("Invalid unary operation: {op} {operand}")]
    InvalidUnaryOp { op: String, operand: String },

    #[error("Unsupported expression: {0}")]
    UnsupportedExpression(String),

    #[error("Aggregate function {0} not allowed in WHERE clause")]
    AggregateInWhere(String),

    #[error("Column {0} must appear in GROUP BY or be used in an aggregate function")]
    ColumnNotInGroupBy(String),

    #[error("Subqueries are not yet supported")]
    SubqueriesNotSupported,

    #[error("JOINs are not yet supported")]
    JoinsNotSupported,

    #[error("Invalid path literal: {0}")]
    InvalidPath(String),

    #[error("Invalid JSON literal: {0}")]
    InvalidJson(String),

    #[error("Invalid LIMIT value: {0}")]
    InvalidLimit(String),

    #[error("Invalid OFFSET value: {0}")]
    InvalidOffset(String),

    #[error("Internal error: {0}")]
    InternalError(String),

    // ORDER statement errors
    #[error("ORDER: Empty path in node reference")]
    OrderEmptyPath,

    #[error("ORDER: Empty ID in node reference")]
    OrderEmptyId,

    #[error("ORDER: Cannot order a node relative to itself")]
    OrderSelfReference,

    #[error("ORDER: Root node '/' cannot be reordered")]
    OrderRootNodeNotAllowed,

    #[error("ORDER: Invalid node ID format: {0}")]
    OrderInvalidId(String),

    #[error("ORDER: Nodes must be siblings (same parent). Source '{source_path}' and target '{target_path}' have different parents")]
    OrderNotSiblings {
        source_path: String,
        target_path: String,
    },

    // MOVE statement errors
    #[error("MOVE: Cannot move a node into itself")]
    MoveSelfReference,

    #[error("MOVE: Cannot move a node into its own descendant. Source '{source_path}' cannot be moved into '{target_path}'")]
    MoveCircularReference {
        source_path: String,
        target_path: String,
    },

    // COPY statement errors
    #[error("COPY: Cannot copy a node into itself")]
    CopySelfReference,

    #[error("COPY: Cannot copy a node into its own descendant. Source '{source_path}' cannot be copied into '{target_path}'")]
    CopyCircularReference {
        source_path: String,
        target_path: String,
    },

    // TRANSLATE statement errors
    #[error("TRANSLATE: Empty locale code")]
    TranslateEmptyLocale,

    #[error("TRANSLATE: Invalid locale code: {0}")]
    TranslateInvalidLocale(String),

    #[error("TRANSLATE: Empty block UUID")]
    TranslateEmptyBlockUuid,

    // RELATE/UNRELATE statement errors
    #[error("RELATE: Empty path in node reference")]
    RelateEmptyPath,

    #[error("RELATE: Empty ID in node reference")]
    RelateEmptyId,

    #[error("RELATE: Invalid node ID format: {0}")]
    RelateInvalidId(String),

    #[error("RELATE: Cannot create a relationship from a node to itself")]
    RelateSelfReference,
}
