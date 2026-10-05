//! Binding: replace every placeholder of a template with its value's literal.

use super::{locale_marker_param, param_index};
use crate::analyzer::{coerce_text_to_path, AnalyzedStatement, DataType, Expr, Literal, TypedExpr};
use crate::logical_plan::LogicalPlan;

/// Bind `values` (one literal per `$n`, from [`super::param_literal`]) into an
/// analyzed template: its WHERE clause and the locales a `locale = $n` was
/// extracted into. [`super::check`] admitted placeholders nowhere else.
pub fn bind_statement(stmt: &mut AnalyzedStatement, values: &[Literal]) -> Result<(), String> {
    let AnalyzedStatement::Query(q) = stmt else {
        return Err("not a query".into());
    };
    if let Some(selection) = &mut q.selection {
        bind_expr(selection, values)?;
    }
    bind_locales(&mut q.locales, values)
}

/// Bind `values` into a template's logical plan: every expression and every
/// scan's locales. The caller re-runs the value-driven optimizer passes
/// (`Optimizer::rebind`).
pub fn bind_plan(plan: &mut LogicalPlan, values: &[Literal]) -> Result<(), String> {
    let mut result = Ok(());
    plan.for_each_expr_mut(&mut |expr| {
        if result.is_ok() {
            result = bind_expr(expr, values);
        }
    });
    result?;
    let mut result = Ok(());
    plan.for_each_locales_mut(&mut |locales| {
        if result.is_ok() {
            result = bind_locales(locales, values);
        }
    });
    result
}

/// The placeholders left in `plan` — 0 after a complete binding.
pub fn count_parameters(plan: &mut LogicalPlan) -> usize {
    let mut n = 0;
    plan.for_each_expr_mut(&mut |expr| n += parameters_in(expr));
    n
}

fn parameters_in(expr: &TypedExpr) -> usize {
    let own = usize::from(matches!(expr.expr, Expr::Literal(Literal::Parameter(_))));
    let mut n = own;
    expr.for_each_child(&mut |child| n += parameters_in(child));
    n
}

fn bind_expr(expr: &mut TypedExpr, values: &[Literal]) -> Result<(), String> {
    if let Expr::Literal(Literal::Parameter(p)) = &expr.expr {
        let bound = bound_literal(p, &expr.data_type, values)?;
        expr.expr = Expr::Literal(bound);
        return Ok(());
    }
    let mut result = Ok(());
    expr.for_each_child_mut(&mut |child| {
        if result.is_ok() {
            result = bind_expr(child, values);
        }
    });
    result
}

/// The literal `$n` takes where it was typed `data_type`: the value's own
/// literal, or — where the analyzer coerced it to a Path — the same coercion
/// a Text literal gets (`coerce_text_to_path`). Any other mismatch means the
/// value is not the one the template was analyzed for: an error, and the
/// statement is planned from its text instead.
fn bound_literal(p: &str, data_type: &DataType, values: &[Literal]) -> Result<Literal, String> {
    let value = param_index(p)
        .and_then(|i| values.get(i))
        .ok_or_else(|| format!("parameter {p} has no value"))?;
    if matches!(data_type, DataType::Path) {
        if let Literal::Text(s) = value {
            return coerce_text_to_path(s).map_err(|e| e.to_string());
        }
    }
    if &value.data_type() == data_type {
        Ok(value.clone())
    } else {
        Err(format!(
            "parameter {p} was analyzed as {data_type}, bound to {}",
            value.data_type()
        ))
    }
}

fn bind_locales(locales: &mut [String], values: &[Literal]) -> Result<(), String> {
    for locale in locales {
        let Some(p) = locale_marker_param(locale) else {
            continue;
        };
        let value = param_index(p)
            .and_then(|i| values.get(i))
            .ok_or_else(|| format!("parameter {p} has no value"))?;
        let Literal::Text(code) = value else {
            return Err(format!("locale parameter {p} is not text"));
        };
        *locale = code.clone();
    }
    Ok(())
}
