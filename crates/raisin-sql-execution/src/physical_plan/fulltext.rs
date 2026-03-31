//! Full-Text Search Execution
//!
//! Implements full-text search using Tantivy with PostgreSQL-compatible syntax.
//! Parses PostgreSQL FTS expressions and converts them to Tantivy queries.

use super::executor::{ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use super::scan_executors::node_to_row;
use async_stream::try_stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::fulltext::{FullTextSearchQuery, IndexingEngine};
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a FullTextScan operator
///
/// Queries the Tantivy full-text index and returns matching nodes ranked by relevance.
/// This handles PostgreSQL-style full-text search syntax:
/// ```sql
/// WHERE to_tsvector('english', content) @@ to_tsquery('english', 'search & query')
/// ```
pub async fn execute_fulltext_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (
        tenant_id,
        repo_id,
        branch,
        workspace,
        table,
        alias,
        language,
        query_str,
        limit,
        projection,
    ) = match plan {
        PhysicalPlan::FullTextScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            language,
            query,
            limit,
            projection,
            ..
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            language.clone(),
            query.clone(),
            *limit,
            projection.clone(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for full-text scan".to_string(),
            ))
        }
    };

    // Check if indexing engine is available
    let indexing_engine = ctx
        .indexing_engine
        .as_ref()
        .ok_or_else(|| {
            Error::Validation("Full-text search requires an indexing engine".to_string())
        })?
        .clone();

    let storage = ctx.storage.clone();
    let ctx_clone = ctx.clone(); // Clone context for use in async stream

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        // Convert PostgreSQL query syntax to Tantivy if needed
        let tantivy_query = convert_postgres_query(&query_str);

        // Build search query
        // FULLTEXT_MATCH uses workspace from table context (single workspace)
        let search_query = FullTextSearchQuery {
            tenant_id: tenant_id.clone(),
            repo_id: repo_id.clone(),
            branch: branch.clone(),
            workspace_ids: Some(vec![workspace.clone()]),
            language: language.clone(),
            query: tantivy_query,
            limit, // Already usize, not Option
            revision: ctx_clone.max_revision, // Point-in-time search: None = HEAD/latest
        };

        // Execute search (not async)
        let results = indexing_engine.search(&search_query)?;

        // For each result, fetch the full node and create a row
        for result in results {
            // Fetch the node from storage (using same revision as search)
            if let Some(node) = storage
                .nodes()
                .get(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), &result.node_id, ctx_clone.max_revision.as_ref())
                .await?
            {
                // Skip root nodes (system detail that users shouldn't see)
                if node.path == "/" {
                    continue;
                }

                let mut row = node_to_row(&node, &qualifier, &workspace, &projection, &ctx_clone, "en").await?;

                // Add pseudo-columns for full-text search metadata
                row.insert(
                    "_ts_rank".to_string(),
                    PropertyValue::Float(result.score as f64),
                );

                yield row;
            }
        }
    }))
}

/// Convert PostgreSQL FTS query syntax to Tantivy query syntax
///
/// PostgreSQL operators:
/// - `&` (AND) → Tantivy AND
/// - `|` (OR) → Tantivy OR
/// - `!` (NOT) → Tantivy NOT
/// - `:*` (prefix) → Tantivy `*` wildcard
///
/// # Examples
///
/// ```text
/// "search & query"        → "search AND query"
/// "rust | go"             → "rust OR go"
/// "database & !sql"       → "database AND NOT sql"
/// "post:*"                → "post*"
/// ```
pub fn convert_postgres_query(pg_query: &str) -> String {
    let mut result = pg_query.to_string();

    // Replace PostgreSQL operators with Tantivy equivalents
    // Note: Order matters - do longest matches first
    result = result.replace(":*", "*"); // Prefix operator
    result = result.replace('&', " AND ");
    result = result.replace('|', " OR ");
    result = result.replace('!', " NOT ");

    // Clean up extra spaces
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse a PostgreSQL to_tsquery expression to extract the query string
///
/// Handles patterns like:
/// - `to_tsquery('search query')`
/// - `to_tsquery('english', 'search query')`
///
/// Returns the query string and optionally the language.
pub fn parse_tsquery(expr: &str) -> Result<(Option<String>, String), Error> {
    let expr = expr.trim();

    // Look for to_tsquery(...) pattern
    if !expr.starts_with("to_tsquery(") {
        return Err(Error::Validation(
            "Expected to_tsquery() function".to_string(),
        ));
    }

    // Extract arguments between parentheses
    let args_start = expr.find('(').unwrap() + 1;
    let args_end = expr
        .rfind(')')
        .ok_or_else(|| Error::Validation("Unclosed to_tsquery()".to_string()))?;
    let args = &expr[args_start..args_end];

    // Split by comma, handling quoted strings
    let parts = split_sql_args(args);

    match parts.len() {
        1 => {
            // Single argument: query only
            let query = unquote(&parts[0]);
            Ok((None, query))
        }
        2 => {
            // Two arguments: language and query
            let language = unquote(&parts[0]);
            let query = unquote(&parts[1]);
            Ok((Some(language), query))
        }
        _ => Err(Error::Validation(
            "to_tsquery() requires 1 or 2 arguments".to_string(),
        )),
    }
}

/// Parse a PostgreSQL to_tsvector expression to extract the language
///
/// Handles patterns like:
/// - `to_tsvector('content')`
/// - `to_tsvector('english', content)`
pub fn parse_tsvector(expr: &str) -> Result<Option<String>, Error> {
    let expr = expr.trim();

    if !expr.starts_with("to_tsvector(") {
        return Err(Error::Validation(
            "Expected to_tsvector() function".to_string(),
        ));
    }

    let args_start = expr.find('(').unwrap() + 1;
    let args_end = expr
        .rfind(')')
        .ok_or_else(|| Error::Validation("Unclosed to_tsvector()".to_string()))?;
    let args = &expr[args_start..args_end];

    let parts = split_sql_args(args);

    match parts.len() {
        1 => Ok(None), // No language specified
        2 => {
            let language = unquote(&parts[0]);
            Ok(Some(language))
        }
        _ => Err(Error::Validation(
            "to_tsvector() requires 1 or 2 arguments".to_string(),
        )),
    }
}

/// Split SQL function arguments, respecting quoted strings
fn split_sql_args(args: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in args.chars() {
        match ch {
            '\'' | '"' if !in_quotes => {
                in_quotes = true;
                quote_char = ch;
                current.push(ch);
            }
            c if c == quote_char && in_quotes => {
                in_quotes = false;
                current.push(ch);
            }
            ',' if !in_quotes => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

/// Remove surrounding quotes from a string
fn unquote(s: &str) -> String {
    let trimmed = s.trim();
    if (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('"') && trimmed.ends_with('"'))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_postgres_query_and() {
        let pg = "search & query";
        let tantivy = convert_postgres_query(pg);
        assert_eq!(tantivy, "search AND query");
    }

    #[test]
    fn test_convert_postgres_query_or() {
        let pg = "rust | go";
        let tantivy = convert_postgres_query(pg);
        assert_eq!(tantivy, "rust OR go");
    }

    #[test]
    fn test_convert_postgres_query_not() {
        let pg = "database & !sql";
        let tantivy = convert_postgres_query(pg);
        assert_eq!(tantivy, "database AND NOT sql");
    }

    #[test]
    fn test_convert_postgres_query_prefix() {
        let pg = "post:*";
        let tantivy = convert_postgres_query(pg);
        assert_eq!(tantivy, "post*");
    }

    #[test]
    fn test_convert_postgres_query_complex() {
        let pg = "(rust | go) & web & !php";
        let tantivy = convert_postgres_query(pg);
        assert_eq!(tantivy, "(rust OR go) AND web AND NOT php");
    }

    #[test]
    fn test_parse_tsquery_simple() {
        let expr = "to_tsquery('search query')";
        let (lang, query) = parse_tsquery(expr).unwrap();
        assert_eq!(lang, None);
        assert_eq!(query, "search query");
    }

    #[test]
    fn test_parse_tsquery_with_language() {
        let expr = "to_tsquery('english', 'search query')";
        let (lang, query) = parse_tsquery(expr).unwrap();
        assert_eq!(lang, Some("english".to_string()));
        assert_eq!(query, "search query");
    }

    #[test]
    fn test_parse_tsvector_simple() {
        let expr = "to_tsvector('content')";
        let lang = parse_tsvector(expr).unwrap();
        assert_eq!(lang, None);
    }

    #[test]
    fn test_parse_tsvector_with_language() {
        let expr = "to_tsvector('english', content)";
        let lang = parse_tsvector(expr).unwrap();
        assert_eq!(lang, Some("english".to_string()));
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("'hello'"), "hello");
        assert_eq!(unquote("\"world\""), "world");
        assert_eq!(unquote("test"), "test");
        assert_eq!(unquote("  'spaced'  "), "spaced");
    }

    #[test]
    fn test_split_sql_args_simple() {
        let args = "'english', 'search query'";
        let parts = split_sql_args(args);
        assert_eq!(parts, vec!["'english'", "'search query'"]);
    }

    #[test]
    fn test_split_sql_args_with_comma_in_string() {
        let args = "'hello, world', 'test'";
        let parts = split_sql_args(args);
        assert_eq!(parts, vec!["'hello, world'", "'test'"]);
    }
}
