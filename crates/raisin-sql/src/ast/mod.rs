// TODO(v0.2): Update nom parsers to use direct tuple syntax instead of sequence::tuple
#![allow(deprecated)]

//! Abstract Syntax Tree (AST) module for RaisinSQL
//!
//! This module handles parsing SQL statements and validating RaisinDB-specific constructs.
//!
//! # Main Components
//!
//! - **parser**: Core parsing logic for RaisinSQL statements
//! - **dialect**: RaisinDialect implementation extending PostgreSQL syntax
//! - **functions**: Validation for RaisinDB-specific functions
//! - **error**: Error types for parsing and validation
//! - **ddl**: DDL AST types for schema management (CREATE/ALTER/DROP)
//! - **ddl_parser**: DDL parser using nom combinators
//! - **order**: ORDER statement AST types for node sibling positioning
//! - **order_parser**: ORDER statement parser using nom combinators
//! - **translate**: TRANSLATE statement AST types for locale-aware updates
//! - **translate_parser**: TRANSLATE statement parser using nom combinators
//! - **branch**: BRANCH statement AST types for branch management
//! - **branch_parser**: BRANCH statement parser using nom combinators
//! - **pgq**: SQL/PGQ (ISO SQL:2023) AST types for property graph queries
//! - **pgq_parser**: GRAPH_TABLE parser using nom combinators

pub mod acl;
pub mod acl_parser;
pub mod branch;
pub mod branch_parser;
pub mod copy_parser;
pub mod copy_stmt;
pub mod ddl;
pub mod ddl_keywords;
pub mod ddl_parser;
pub mod dialect;
pub mod error;
pub mod functions;
pub mod move_parser;
pub mod move_stmt;
pub mod order;
pub mod order_parser;
pub mod parser;
pub mod pgq;
pub mod pgq_parser;
pub mod relate;
pub mod relate_parser;
pub mod restore;
pub mod restore_parser;
pub mod transaction;
pub mod transaction_parser;
pub mod translate;
pub mod translate_parser;

// Re-export commonly used types and functions
pub use acl::{AclStatement, Operation, PermissionGrant};
pub use acl_parser::{is_acl_statement, parse_acl};
pub use branch::{
    AlterBranch, BranchAlteration, BranchStatement, CreateBranch, DropBranch, MergeBranch,
    MergeStrategy, RevisionRef,
};
pub use branch_parser::{is_branch_statement, parse_branch, BranchParseError};
pub use copy_parser::{is_copy_statement, parse_copy};
pub use copy_stmt::CopyStatement;
pub use ddl::DdlStatement;
pub use ddl_keywords::{DdlKeywords, KeywordCategory, KeywordInfo};
pub use ddl_parser::parse_ddl;
pub use dialect::RaisinDialect;
pub use error::{ParseError, Result};
pub use functions::{validate_raisin_functions, RaisinFunction};
pub use move_parser::{is_move_statement, parse_move};
pub use move_stmt::MoveStatement;
pub use order::{NodeReference, OrderPosition, OrderStatement};
pub use order_parser::{is_order_statement, parse_order};
pub use parser::{parse_sql, validate_statement};
pub use relate::{RelateEndpoint, RelateNodeReference, RelateStatement, UnrelateStatement};
pub use relate_parser::{is_relate_statement, is_unrelate_statement, parse_relate, parse_unrelate};
pub use transaction::TransactionStatement;
pub use transaction_parser::{is_transaction_statement, parse_transaction};
pub use translate::{
    TranslateFilter, TranslateStatement, TranslationAssignment, TranslationPath, TranslationValue,
};
pub use translate_parser::{is_translate_statement, parse_translate};

// RESTORE statement
pub use restore::RestoreStatement;
pub use restore_parser::{is_restore_statement, parse_restore};

// SQL/PGQ (ISO SQL:2023) - Property Graph Queries
pub use pgq::{
    is_system_field, BinaryOperator, ColumnExpr, ColumnsClause, Direction, Expr, GraphTableQuery,
    Literal, MatchClause, NodePattern, PathPattern, PathQuantifier, PatternElement,
    RelationshipPattern, SourceSpan, UnaryOperator, WhereClause, DEFAULT_GRAPH_NAME, SYSTEM_FIELDS,
};
pub use pgq_parser::{
    extract_graph_table_arg, find_graph_tables, is_graph_table_expression, parse_graph_table,
    preprocess_graph_tables, PgqParseError,
};
