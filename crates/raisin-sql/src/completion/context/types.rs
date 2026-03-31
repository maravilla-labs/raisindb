//! SQL context types for completion
//!
//! Defines the semantic context variants at a cursor position and the
//! analyzed context metadata used by the completion provider.

use std::collections::HashMap;

/// SQL context at cursor position
#[derive(Debug, Clone, PartialEq)]
pub enum SqlContext {
    /// Start of a new statement (empty or after semicolon)
    StatementStart,
    /// After SELECT keyword, expecting columns
    SelectClause,
    /// After FROM keyword, expecting table name
    FromClause,
    /// After JOIN keyword, expecting table name
    JoinClause,
    /// After ON keyword in JOIN, expecting condition
    JoinCondition,
    /// After WHERE keyword, expecting expression
    WhereClause,
    /// After GROUP BY keywords, expecting columns
    GroupByClause,
    /// After HAVING keyword, expecting expression
    HavingClause,
    /// After ORDER BY keywords, expecting columns
    OrderByClause,
    /// After SET keyword in UPDATE, expecting assignments
    SetClause,
    /// After INSERT INTO table, expecting column list or VALUES
    InsertClause,
    /// Inside VALUES(...), expecting values
    ValuesClause,
    /// After table alias followed by dot (e.g., "n.")
    AfterDot {
        /// The alias or table name before the dot
        qualifier: String,
    },
    /// Inside function parentheses
    FunctionArgument {
        /// Name of the function
        function_name: String,
        /// 0-based index of current argument
        arg_index: usize,
    },
    /// After CREATE keyword
    AfterCreate,
    /// After ALTER keyword
    AfterAlter,
    /// After DROP keyword
    AfterDrop,
    /// Inside PROPERTIES(...) in DDL
    PropertyDefinition,
    /// Expecting a property type name
    PropertyType,
    /// After ORDER table SET path='...' - expecting ABOVE/BELOW
    OrderPosition,
    /// After MOVE table SET path='...' - expecting TO
    MoveTarget,
    /// Generic expression context
    Expression,
    /// Unknown context (fallback to keywords)
    Unknown,
}

/// Table alias mapping
#[derive(Debug, Clone)]
pub struct TableAlias {
    /// The alias name used in the query
    pub alias: String,
    /// The actual table name
    pub table_name: String,
}

/// Analyzed context with additional metadata
#[derive(Debug, Clone)]
pub struct AnalyzedContext {
    /// The detected SQL context
    pub context: SqlContext,
    /// Table aliases defined in the query
    pub aliases: HashMap<String, String>,
    /// Tables referenced in FROM clause
    pub from_tables: Vec<String>,
    /// Whether we're inside a transaction block
    pub in_transaction: bool,
    /// The partial word at cursor (for filtering)
    pub partial_word: String,
}

impl Default for AnalyzedContext {
    fn default() -> Self {
        Self {
            context: SqlContext::Unknown,
            aliases: HashMap::new(),
            from_tables: Vec::new(),
            in_transaction: false,
            partial_word: String::new(),
        }
    }
}
