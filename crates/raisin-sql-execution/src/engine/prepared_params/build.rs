//! Building a template and proving it against the substituted text.

use super::{Template, RECENT_PER_TEMPLATE};
use crate::engine::prepared::{self, Prepared};
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer, Catalog, DataType, Literal};
use raisin_sql::template;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Build the template of `sql` and prove it against `planned`, the
/// statement(s) the substituted text prepared to.
pub(super) fn build(
    catalog: &Arc<dyn Catalog>,
    batch: bool,
    sql: &str,
    rendered: &[String],
    values: &[Literal],
    planned: &[Arc<Prepared>],
) -> Result<Template, String> {
    let [planned] = planned else {
        return Err("a multi-statement batch".into());
    };
    if !template::placeholders_are_tokens(sql) {
        return Err("a `$n` the substitution rewrites is not a placeholder token".into());
    }
    let types: Arc<[DataType]> = values.iter().map(Literal::data_type).collect();
    let analyzer = Analyzer::with_catalog_arc(catalog.clone());
    let analyzed = if batch {
        let mut statements = analyzer
            .analyze_batch_template(sql, types)
            .map_err(|e| e.to_string())?;
        if statements.len() != 1 {
            return Err("a multi-statement batch".into());
        }
        statements.pop().expect("one statement")
    } else {
        analyzer
            .analyze_template(sql, types)
            .map_err(|e| e.to_string())?
    };
    if crate::engine::subquery_bind::statement_needs_binding(&analyzed) {
        return Err("subqueries are bound per execution".into());
    }
    template::check(&analyzed)?;
    let plan = match &analyzed {
        AnalyzedStatement::Query(q) if !q.from.is_empty() => {
            Some(prepared::logical_plan(catalog, &analyzed).map_err(|e| e.to_string())?)
        }
        _ => None,
    };
    let template = Template {
        analyzed,
        plan,
        recent: Mutex::new(VecDeque::new()),
        _catalog: Arc::downgrade(catalog),
    };

    // The proof: bound to this execution's values, the template must be the
    // statement the text prepared to — analysis and optimized plan alike.
    let (analyzed, plan) = template.bind_parts(values)?;
    let same_query = match (&analyzed, &planned.analyzed) {
        (AnalyzedStatement::Query(a), AnalyzedStatement::Query(b)) => a == b,
        _ => false,
    };
    if !same_query || plan != planned.plan {
        return Err("bound template differs from the substituted text's plan".into());
    }
    if format!("{analyzed:?}").contains("Parameter(\"") {
        return Err("a placeholder survived binding".into());
    }
    if let Some(mut plan) = plan {
        if template::count_parameters(&mut plan) != 0 {
            return Err("a placeholder survived binding".into());
        }
    }
    // The text's statement IS the bound one: keep it as the first recent.
    template
        .recent
        .lock()
        .map_err(|_| "poisoned".to_string())?
        .push_front((rendered.join("\u{1}"), planned.clone()));
    Ok(template)
}
