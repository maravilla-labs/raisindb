//! Semantic Analysis Module
//!
//! This module performs semantic analysis on parsed SQL ASTs, producing typed and validated
//! representations of queries.
//!
//! # Main Components
//!
//! - **types**: Type system with coercion rules
//! - **catalog**: Schema catalog for name resolution
//! - **functions**: Function registry with type signatures
//! - **typed_expr**: Typed expression tree
//! - **semantic**: Core semantic analysis logic
//! - **error**: Semantic analysis errors

use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser as SqlParser;

use crate::ast::{is_graph_table_expression, preprocess_graph_tables};

pub mod catalog;
pub mod error;
pub mod functions;
mod helpers;
pub mod pg_catalog;
pub mod semantic;
mod statement_analysis;
pub mod typed_expr;
pub mod types;

// Re-export commonly used types
pub use catalog::{Catalog, ColumnDef, StaticCatalog, TableDef};
pub use error::{AnalysisError, Result};
pub use functions::{FunctionCategory, FunctionRegistry, FunctionSignature};
pub use semantic::{
    AnalyzedCopy, AnalyzedDelete, AnalyzedDistinct, AnalyzedInsert, AnalyzedMove, AnalyzedOrder,
    AnalyzedQuery, AnalyzedRelate, AnalyzedRelateEndpoint, AnalyzedRestore, AnalyzedShow,
    AnalyzedStatement, AnalyzedTranslate, AnalyzedTranslateFilter, AnalyzedTranslationValue,
    AnalyzedUnrelate, AnalyzedUpdate, DmlTableTarget, ExplainFormat, ExplainStatement, JoinInfo,
    JoinType, TableRef,
};
pub use typed_expr::{
    BinaryOperator, Expr, FrameBound, FrameMode, Literal, TypedExpr, UnaryOperator, WindowFrame,
    WindowFunction,
};
pub use types::DataType;

/// Semantic analyzer
pub struct Analyzer {
    catalog: Box<dyn Catalog>,
    functions: FunctionRegistry,
}

impl Analyzer {
    /// Create analyzer with default catalog and function registry
    pub fn new() -> Self {
        Self {
            catalog: Box::new(StaticCatalog::default_nodes_schema()),
            functions: FunctionRegistry::default(),
        }
    }

    /// Create analyzer with custom catalog
    pub fn with_catalog(catalog: Box<dyn Catalog>) -> Self {
        Self {
            catalog,
            functions: FunctionRegistry::default(),
        }
    }

    /// Analyze a SQL string
    pub fn analyze(&self, sql: &str) -> Result<AnalyzedStatement> {
        // 1. Try transaction parser first (BEGIN, COMMIT)
        tracing::debug!("   Checking for transaction statements...");
        if crate::ast::transaction_parser::is_transaction_statement(sql) {
            match crate::ast::transaction_parser::parse_transaction(sql) {
                Ok((remaining, txn_stmt)) => {
                    // Verify we consumed all input (except whitespace and semicolon)
                    let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
                    if !remaining_trimmed.is_empty() {
                        return Err(AnalysisError::ParseError(format!(
                            "Unexpected trailing content: '{}'",
                            remaining_trimmed
                        )));
                    }
                    tracing::debug!(
                        "   Transaction statement detected: {:?}",
                        txn_stmt.operation()
                    );
                    return Ok(AnalyzedStatement::Transaction(txn_stmt));
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "Transaction parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2. Try ORDER parser (ORDER path='...' ABOVE/BELOW path='...')
        tracing::debug!("   Checking for ORDER statements...");
        if crate::ast::order_parser::is_order_statement(sql) {
            match crate::ast::order_parser::parse_order(sql) {
                Ok(Some(order_stmt)) => {
                    tracing::debug!("   ORDER statement detected: {}", order_stmt);
                    // Analyze the ORDER statement (validate references)
                    let analyzed =
                        statement_analysis::analyze_order(self.catalog.as_ref(), &order_stmt)?;
                    return Ok(AnalyzedStatement::Order(analyzed));
                }
                Ok(None) => {
                    // Not an ORDER statement, continue
                    tracing::debug!("   Not an ORDER statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "ORDER parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2b. Try MOVE parser (MOVE Table SET path='...' TO path='...')
        tracing::debug!("   Checking for MOVE statements...");
        if crate::ast::move_parser::is_move_statement(sql) {
            match crate::ast::move_parser::parse_move(sql) {
                Ok(Some(move_stmt)) => {
                    tracing::debug!("   MOVE statement detected: {}", move_stmt);
                    // Analyze the MOVE statement (validate references)
                    let analyzed =
                        statement_analysis::analyze_move(self.catalog.as_ref(), &move_stmt)?;
                    return Ok(AnalyzedStatement::Move(analyzed));
                }
                Ok(None) => {
                    // Not a MOVE statement, continue
                    tracing::debug!("   Not a MOVE statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "MOVE parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2b2. Try COPY parser (COPY [TREE] Table SET path='...' TO path='...' [AS 'name'])
        tracing::debug!("   Checking for COPY statements...");
        if crate::ast::copy_parser::is_copy_statement(sql) {
            match crate::ast::copy_parser::parse_copy(sql) {
                Ok(Some(copy_stmt)) => {
                    tracing::debug!("   COPY statement detected: {}", copy_stmt);
                    // Analyze the COPY statement (validate references)
                    let analyzed =
                        statement_analysis::analyze_copy(self.catalog.as_ref(), &copy_stmt)?;
                    return Ok(AnalyzedStatement::Copy(analyzed));
                }
                Ok(None) => {
                    // Not a COPY statement, continue
                    tracing::debug!("   Not a COPY statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "COPY parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2c. Try TRANSLATE parser (UPDATE Table FOR LOCALE 'xx' SET ...)
        tracing::debug!("   Checking for TRANSLATE statements...");
        if crate::ast::translate_parser::is_translate_statement(sql) {
            match crate::ast::translate_parser::parse_translate(sql) {
                Ok(Some(translate_stmt)) => {
                    tracing::debug!("   TRANSLATE statement detected: {}", translate_stmt);
                    // Analyze the TRANSLATE statement (validate locale, paths)
                    let analyzed = statement_analysis::analyze_translate(
                        self.catalog.as_ref(),
                        &translate_stmt,
                    )?;
                    return Ok(AnalyzedStatement::Translate(analyzed));
                }
                Ok(None) => {
                    // Not a TRANSLATE statement, continue
                    tracing::debug!("   Not a TRANSLATE statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "TRANSLATE parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2d. Try RELATE parser (RELATE FROM ... TO ...)
        tracing::debug!("   Checking for RELATE statements...");
        if crate::ast::relate_parser::is_relate_statement(sql) {
            match crate::ast::relate_parser::parse_relate(sql) {
                Ok(Some(relate_stmt)) => {
                    tracing::debug!("   RELATE statement detected");
                    let analyzed = statement_analysis::analyze_relate(&relate_stmt)?;
                    return Ok(AnalyzedStatement::Relate(analyzed));
                }
                Ok(None) => {
                    tracing::debug!("   Not a RELATE statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "RELATE parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2e. Try UNRELATE parser (UNRELATE FROM ... TO ...)
        tracing::debug!("   Checking for UNRELATE statements...");
        if crate::ast::relate_parser::is_unrelate_statement(sql) {
            match crate::ast::relate_parser::parse_unrelate(sql) {
                Ok(Some(unrelate_stmt)) => {
                    tracing::debug!("   UNRELATE statement detected");
                    let analyzed = statement_analysis::analyze_unrelate(&unrelate_stmt)?;
                    return Ok(AnalyzedStatement::Unrelate(analyzed));
                }
                Ok(None) => {
                    tracing::debug!("   Not an UNRELATE statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "UNRELATE parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2f. Try BRANCH parser (CREATE/DROP/ALTER/MERGE BRANCH, USE/CHECKOUT BRANCH, SHOW BRANCHES)
        tracing::debug!("   Checking for BRANCH statements...");
        if crate::ast::branch_parser::is_branch_statement(sql) {
            match crate::ast::branch_parser::parse_branch(sql) {
                Ok(Some(branch_stmt)) => {
                    tracing::debug!("   BRANCH statement detected: {}", branch_stmt.operation());
                    // Branch statements are passed through directly - validation happens at execution
                    return Ok(AnalyzedStatement::Branch(branch_stmt));
                }
                Ok(None) => {
                    tracing::debug!("   Not a BRANCH statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "BRANCH parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2g. Try RESTORE parser (RESTORE [TREE] NODE ... TO REVISION ...)
        tracing::debug!("   Checking for RESTORE statements...");
        if crate::ast::restore_parser::is_restore_statement(sql) {
            match crate::ast::restore_parser::parse_restore(sql) {
                Ok(Some(restore_stmt)) => {
                    tracing::debug!("   RESTORE statement detected: {}", restore_stmt);
                    let analyzed = statement_analysis::analyze_restore(&restore_stmt)?;
                    return Ok(AnalyzedStatement::Restore(analyzed));
                }
                Ok(None) => {
                    tracing::debug!("   Not a RESTORE statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "RESTORE parse error: {}",
                        e
                    )));
                }
            }
        }

        // 2h. Check for ACL statements (CREATE/ALTER/DROP ROLE/GROUP/USER, GRANT/REVOKE)
        tracing::debug!("   Checking for ACL statements...");
        if crate::ast::acl_parser::is_acl_statement(sql) {
            match crate::ast::acl_parser::parse_acl(sql) {
                Ok(Some(acl_stmt)) => {
                    tracing::debug!("   ACL statement detected: {}", acl_stmt.operation());
                    return Ok(AnalyzedStatement::Acl(acl_stmt));
                }
                Ok(None) => {
                    tracing::debug!("   Not an ACL statement, continuing...");
                }
                Err(e) => {
                    return Err(AnalysisError::ParseError(format!(
                        "ACL parse error: {}",
                        e.message
                    )));
                }
            }
        }

        // 3. Try DDL parser (CREATE/ALTER/DROP NODETYPE/ARCHETYPE/ELEMENTTYPE)
        tracing::debug!("   Checking for DDL statements...");
        match crate::ast::ddl_parser::parse_ddl(sql) {
            Ok(Some(ddl_stmt)) => {
                tracing::debug!("   DDL statement detected: {:?}", ddl_stmt.operation());
                // DDL statements are already validated by the parser
                // No additional semantic analysis needed at this stage
                return Ok(AnalyzedStatement::Ddl(ddl_stmt));
            }
            Ok(None) => {
                // Not a DDL statement, continue with regular SQL parsing
                tracing::debug!("   Not a DDL statement, continuing with SQL parser...");
            }
            Err(e) => {
                // DDL parsing failed
                return Err(AnalysisError::ParseError(format!("DDL parse error: {}", e)));
            }
        }

        // 2. Check for UPSERT and convert to INSERT
        // UPSERT has identical syntax to INSERT, only the execution semantics differ
        // (UPSERT uses put_node which creates or updates, INSERT uses add_node which fails if exists)
        let trimmed = sql.trim();
        let is_upsert = trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("UPSERT");
        let sql_after_upsert = if is_upsert {
            tracing::debug!("   UPSERT detected, converting to INSERT for parsing");
            // Replace "UPSERT" with "INSERT" (preserving the rest of the statement)
            format!("INSERT{}", &trimmed[6..])
        } else {
            sql.to_string()
        };

        // 2b. Preprocess GRAPH_TABLE expressions
        // GRAPH_TABLE content uses PGQ syntax that sqlparser-rs doesn't understand.
        // Convert: GRAPH_TABLE(MATCH ... COLUMNS ...)
        // To:      GRAPH_TABLE('GRAPH_TABLE(MATCH ... COLUMNS ...)')
        // The string content is then parsed by our PGQ parser at execution time.
        let sql_to_parse = if is_graph_table_expression(&sql_after_upsert) {
            tracing::debug!("   GRAPH_TABLE detected, preprocessing for sqlparser");
            preprocess_graph_tables(&sql_after_upsert)
        } else {
            sql_after_upsert
        };

        // 3. Parse SQL to AST (without validation)
        tracing::debug!("   Parsing SQL with PostgreSQL dialect...");
        let dialect = PostgreSqlDialect {};
        let statements = SqlParser::parse_sql(&dialect, &sql_to_parse)
            .map_err(|e| AnalysisError::ParseError(e.to_string()))?;

        tracing::debug!(
            "   SQL parsed successfully, {} statements found",
            statements.len()
        );

        if statements.is_empty() {
            return Err(AnalysisError::EmptyStatement);
        }

        if statements.len() > 1 {
            return Err(AnalysisError::MultipleStatements);
        }

        // 4. Analyze statement
        tracing::debug!("   Performing semantic analysis...");
        let mut context = semantic::AnalyzerContext::new(self.catalog.as_ref(), &self.functions);
        // Set upsert flag before analysis so analyze_insert knows it's an UPSERT
        context.set_upsert(is_upsert);
        let result = context.analyze_statement(&statements[0])?;
        tracing::debug!("   Semantic analysis complete");
        Ok(result)
    }

    /// Analyze multiple SQL statements in a batch
    ///
    /// This method supports executing multiple statements separated by semicolons,
    /// which is essential for transaction blocks like:
    /// ```sql
    /// BEGIN;
    /// UPDATE nodes SET properties = '{}' WHERE id = '...';
    /// COMMIT WITH MESSAGE 'Updated node';
    /// ```
    pub fn analyze_batch(&self, sql: &str) -> Result<Vec<AnalyzedStatement>> {
        tracing::debug!("Analyzing batch SQL: {} chars", sql.len());

        // Split SQL by semicolons, respecting string literals
        let statements_sql = helpers::split_sql_statements(sql);

        if statements_sql.is_empty() {
            return Err(AnalysisError::EmptyStatement);
        }

        tracing::debug!("   Found {} statements in batch", statements_sql.len());

        let mut results = Vec::new();
        for (idx, stmt_sql) in statements_sql.iter().enumerate() {
            let stmt_sql = stmt_sql.trim();
            if stmt_sql.is_empty() {
                continue;
            }

            tracing::debug!(
                "   Analyzing statement {}: '{}'",
                idx + 1,
                if stmt_sql.len() > 50 {
                    &stmt_sql[..50]
                } else {
                    stmt_sql
                }
            );

            // Analyze each statement using the existing single-statement analyzer
            let analyzed = self.analyze(stmt_sql)?;
            results.push(analyzed);
        }

        if results.is_empty() {
            return Err(AnalysisError::EmptyStatement);
        }

        tracing::debug!("   Batch analysis complete: {} statements", results.len());
        Ok(results)
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
