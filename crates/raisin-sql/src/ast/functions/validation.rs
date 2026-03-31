//! RaisinDB function validation
//!
//! Validates custom function calls within SQL queries, checking
//! argument counts and recursively validating nested expressions.

use sqlparser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, Query, Select, SelectItem, SetExpr, TableFactor,
    TableWithJoins,
};

use super::registry::RaisinFunction;
use crate::ast::error::{ParseError, Result};

/// Validate RaisinDB-specific functions in a query
pub fn validate_raisin_functions(query: &Query) -> Result<()> {
    // Validate functions in the SELECT body
    if let SetExpr::Select(select) = &*query.body {
        validate_select(select)?;
    }

    Ok(())
}

/// Validate table names in FROM clause
pub(crate) fn validate_table_names_in_query(query: &Query) -> Result<()> {
    if let SetExpr::Select(select) = &*query.body {
        for table_with_joins in &select.from {
            validate_table_names_in_factor(&table_with_joins.relation)?;
            for join in &table_with_joins.joins {
                validate_table_names_in_factor(&join.relation)?;
            }
        }
    }
    Ok(())
}

/// Validate functions in a SELECT statement
fn validate_select(select: &Select) -> Result<()> {
    // Validate functions in SELECT list
    for item in &select.projection {
        validate_select_item(item)?;
    }

    // Validate functions in FROM clause (table-valued functions)
    for table in &select.from {
        validate_table_with_joins(table)?;
    }

    // Validate functions in WHERE clause
    if let Some(ref selection) = select.selection {
        validate_expression(selection)?;
    }

    Ok(())
}

/// Validate that table factors reference 'nodes' table
fn validate_table_names_in_factor(table: &TableFactor) -> Result<()> {
    match table {
        TableFactor::Table { name, .. } => {
            let table_str = name.to_string().to_uppercase();
            // Check if this is a table-valued function
            let is_tvf = RaisinFunction::from_name(&table_str)
                .map(|f| f.is_table_valued())
                .unwrap_or(false);

            if !is_tvf && table_str != "NODES" {
                return Err(ParseError::InvalidTable {
                    operation: "SELECT".to_string(),
                    table: name.to_string(),
                    expected: "nodes".to_string(),
                });
            }
            Ok(())
        }
        TableFactor::Derived { subquery, .. } => {
            validate_table_names_in_query(subquery)?;
            Ok(())
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            validate_table_names_in_factor(&table_with_joins.relation)?;
            for join in &table_with_joins.joins {
                validate_table_names_in_factor(&join.relation)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate functions in SELECT items
fn validate_select_item(item: &SelectItem) -> Result<()> {
    match item {
        SelectItem::UnnamedExpr(expr) => validate_expression(expr),
        SelectItem::ExprWithAlias { expr, .. } => validate_expression(expr),
        SelectItem::Wildcard(_) => Ok(()),
        SelectItem::QualifiedWildcard(_, _) => Ok(()),
    }
}

/// Validate table-valued functions in FROM clause
fn validate_table_with_joins(table: &TableWithJoins) -> Result<()> {
    validate_table_factor(&table.relation)?;

    for join in &table.joins {
        validate_table_factor(&join.relation)?;
    }

    Ok(())
}

/// Validate table factors (including table-valued functions)
fn validate_table_factor(table: &TableFactor) -> Result<()> {
    match table {
        TableFactor::Table { name, .. } => {
            // Check if this is a table-valued function call
            let table_str = name.to_string().to_uppercase();
            if let Some(func) = RaisinFunction::from_name(&table_str) {
                if !func.is_table_valued() {
                    return Err(ParseError::InvalidFunction(format!(
                        "{} is not a table-valued function",
                        table_str
                    )));
                }
            }
            Ok(())
        }
        TableFactor::Derived { subquery, .. } => {
            validate_raisin_functions(subquery)?;
            Ok(())
        }
        TableFactor::TableFunction { expr, .. } => {
            // Validate table function expression
            validate_expression(expr)?;
            Ok(())
        }
        TableFactor::UNNEST { .. } => Ok(()),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            validate_table_with_joins(table_with_joins)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate expressions for RaisinDB functions
fn validate_expression(expr: &Expr) -> Result<()> {
    match expr {
        Expr::Function(func) => validate_function(func),
        Expr::BinaryOp { left, right, .. } => {
            validate_expression(left)?;
            validate_expression(right)?;
            Ok(())
        }
        Expr::UnaryOp { expr, .. } => validate_expression(expr),
        Expr::Nested(expr) => validate_expression(expr),
        Expr::InList { expr, list, .. } => {
            validate_expression(expr)?;
            for item in list {
                validate_expression(item)?;
            }
            Ok(())
        }
        Expr::InSubquery { expr, subquery, .. } => {
            validate_expression(expr)?;
            validate_raisin_functions(subquery)?;
            Ok(())
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            validate_expression(expr)?;
            validate_expression(low)?;
            validate_expression(high)?;
            Ok(())
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(op) = operand {
                validate_expression(op)?;
            }
            for case_when in conditions {
                validate_expression(&case_when.condition)?;
                validate_expression(&case_when.result)?;
            }
            if let Some(else_res) = else_result {
                validate_expression(else_res)?;
            }
            Ok(())
        }
        Expr::Subquery(query) => validate_raisin_functions(query),
        _ => Ok(()),
    }
}

/// Validate a function call
fn validate_function(func: &Function) -> Result<()> {
    let func_name = func.name.to_string();

    // Check if this is a RaisinDB-specific function
    if let Some(raisin_func) = RaisinFunction::from_name(&func_name) {
        // Get the function arguments as a slice
        let args = match &func.args {
            sqlparser::ast::FunctionArguments::None => &[][..],
            sqlparser::ast::FunctionArguments::Subquery(_) => &[][..],
            sqlparser::ast::FunctionArguments::List(ref list) => &list.args[..],
        };

        // Validate argument count
        let actual_args = args.len();
        if !raisin_func.allows_arg_count(actual_args) {
            return Err(ParseError::InvalidFunctionArity {
                function: func_name.clone(),
                expected: raisin_func.arity_description(),
                actual: actual_args,
            });
        }

        // Validate arguments recursively
        for arg in args {
            match arg {
                FunctionArg::Named { arg, .. } => validate_function_arg_expr(arg)?,
                FunctionArg::Unnamed(arg) => validate_function_arg_expr(arg)?,
                FunctionArg::ExprNamed { arg, .. } => validate_function_arg_expr(arg)?,
            }
        }
    }

    Ok(())
}

/// Validate function argument expressions
fn validate_function_arg_expr(arg: &FunctionArgExpr) -> Result<()> {
    match arg {
        FunctionArgExpr::Expr(expr) => validate_expression(expr),
        FunctionArgExpr::Wildcard => Ok(()),
        FunctionArgExpr::QualifiedWildcard(_) => Ok(()),
    }
}
