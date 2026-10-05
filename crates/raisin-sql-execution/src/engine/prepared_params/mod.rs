//! Parameterized statements: ONE prepared template per SQL text with `$n`
//! placeholders, bound to each execution's values (plan Phase 13d).
//!
//! The final-text cache (`prepared.rs`) misses on every new value of a
//! parameterized statement — `WHERE path = $1` for each page of a site is a
//! new text after substitution. Here the cache is keyed on the text BEFORE
//! substitution, plus the type of each value's literal, and the catalog:
//!
//! ```text
//! (catalog identity, single statement | batch, template SQL, value types)
//! ```
//!
//! What binding may and may not do is `raisin_sql::template`'s contract. This
//! side adds the proof: a template is built on its first execution, bound to
//! that execution's values, and compared with the statement planned from the
//! SUBSTITUTED text (which is what that execution then runs). Only an
//! identical result is cached as a template; anything else is remembered as
//! value-dependent and planned per value from then on, exactly as before.
//!
//! Not part of the key, and why that is safe: the caller (RLS, auth, HEAD are
//! applied at execution, after this cache), the branch (planned per
//! execution, `physical_cache.rs`), index state (same), the rendering of a
//! value (its literal TYPE is in the key; the literal itself is bound).
//!
//! A template also keeps its last few bound statements by value, so a
//! repeated execution with the same values reuses the bound statement — and
//! with it its cached physical plan — without binding again.

mod build;

use super::prepared::{self, Prepared, ENTRY_WEIGHT, MAX_SQL_LEN};
use super::ParamFormat;
use build::build;
use raisin_error::Error;
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer, Catalog, DataType, Literal};
use raisin_sql::logical_plan::LogicalPlan;
use raisin_sql::optimizer::Optimizer;
use raisin_sql::template;
use serde_json::Value as JsonValue;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Statements prepared for one execution with parameters.
pub(crate) struct ParamPrepared {
    pub(crate) statements: Vec<Arc<Prepared>>,
    /// The SQL they came from: the template when bound, the substituted text
    /// when planned from it.
    pub(crate) sql: String,
    pub(crate) outcome: ParamOutcome,
}

/// How one execution's statement was prepared — for tests and diagnostics.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamOutcome {
    /// Bound from the statement's cached template.
    Template,
    /// Planned from the text; the template was built (and proved) from it.
    Built,
    /// Planned from the text: the statement's plan depends on its values.
    ValueDependent,
    /// Planned from the text without consulting templates (no parameters,
    /// a value that is not a plain literal, the cache off).
    Text,
}

/// Bound statements a template keeps by value (see the module docs).
const RECENT_PER_TEMPLATE: usize = 4;

enum Entry {
    Template(Template),
    /// Planned per value; the reason, for diagnostics.
    ValueDependent(#[allow(dead_code)] String),
}

struct Template {
    analyzed: AnalyzedStatement,
    plan: Option<LogicalPlan>,
    recent: Mutex<VecDeque<(String, Arc<Prepared>)>>,
    /// Keeps the catalog's allocation reserved while this entry lives.
    _catalog: Weak<dyn Catalog>,
}

/// `(catalog identity, batch, template SQL, value types)`.
type Key = (usize, bool, String, String);

static TEMPLATES: OnceLock<moka::sync::Cache<Key, Arc<Entry>>> = OnceLock::new();
static HITS: AtomicU64 = AtomicU64::new(0);
static BUILT: AtomicU64 = AtomicU64::new(0);
static VALUE_DEPENDENT: AtomicU64 = AtomicU64::new(0);

fn templates() -> &'static moka::sync::Cache<Key, Arc<Entry>> {
    TEMPLATES.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(64 * 1024 * 1024)
            .weigher(|key: &Key, _: &Arc<Entry>| {
                // The template plus the bound statements it may keep.
                (ENTRY_WEIGHT * (1 + RECENT_PER_TEMPLATE as u32))
                    .saturating_add(u32::try_from(key.2.len() * 4).unwrap_or(u32::MAX))
            })
            .build()
    })
}

/// Drop every template (with the final-text cache, `invalidate_plan_cache`).
pub(crate) fn invalidate_templates() {
    if let Some(cache) = TEMPLATES.get() {
        cache.invalidate_all();
    }
}

/// `(template hits, templates built, statements found value-dependent)` since
/// the process started — for tests and diagnostics.
#[doc(hidden)]
pub fn template_cache_stats() -> (u64, u64, u64) {
    (
        HITS.load(Ordering::Relaxed),
        BUILT.load(Ordering::Relaxed),
        VALUE_DEPENDENT.load(Ordering::Relaxed),
    )
}

/// Prepare `sql` with `params`: from its template when it has one, from the
/// substituted text otherwise (which also builds the template on first use).
pub(crate) fn prepare_with_params(
    catalog: &Arc<dyn Catalog>,
    batch: bool,
    sql: &str,
    params: &[JsonValue],
    format: &ParamFormat,
) -> Result<ParamPrepared, Error> {
    let bindable = (!params.is_empty() && prepared::enabled() && sql.len() <= MAX_SQL_LEN)
        .then(|| bind_values(params, format))
        .flatten();
    let Some((rendered, values)) = bindable else {
        return from_text(catalog, batch, sql, params, format, ParamOutcome::Text);
    };
    let key: Key = (
        Arc::as_ptr(catalog) as *const () as usize,
        batch,
        sql.to_string(),
        signature(&values),
    );
    let cache = templates();
    if let Some(entry) = cache.get(&key) {
        if let Entry::Template(template) = entry.as_ref() {
            if let Some(statement) = template.bind(catalog, &rendered, &values) {
                HITS.fetch_add(1, Ordering::Relaxed);
                return Ok(ParamPrepared {
                    statements: vec![statement],
                    sql: sql.to_string(),
                    outcome: ParamOutcome::Template,
                });
            }
        }
        return from_text(
            catalog,
            batch,
            sql,
            params,
            format,
            ParamOutcome::ValueDependent,
        );
    }

    // First use: plan the text (what this execution runs), then build the
    // template and keep it only if it binds to exactly that.
    let mut planned = from_text(catalog, batch, sql, params, format, ParamOutcome::Built)?;
    let entry = match build(catalog, batch, sql, &rendered, &values, &planned.statements) {
        Ok(template) => {
            BUILT.fetch_add(1, Ordering::Relaxed);
            Entry::Template(template)
        }
        Err(reason) => {
            VALUE_DEPENDENT.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(%reason, "SQL template is value-dependent; planned per value");
            planned.outcome = ParamOutcome::ValueDependent;
            Entry::ValueDependent(reason)
        }
    };
    cache.insert(key, Arc::new(entry));
    Ok(planned)
}

/// The statement planned from the substituted text — the behaviour without
/// templates, errors included.
fn from_text(
    catalog: &Arc<dyn Catalog>,
    batch: bool,
    sql: &str,
    params: &[JsonValue],
    format: &ParamFormat,
    outcome: ParamOutcome,
) -> Result<ParamPrepared, Error> {
    let text = raisin_sql::substitute_params_with(sql, params, format)?;
    let statements = if batch {
        prepared::prepare_batch(catalog, &text)?
    } else {
        vec![prepared::prepare_statement(catalog, &text)?]
    };
    Ok(ParamPrepared {
        statements,
        sql: text,
        outcome,
    })
}

/// Each value's rendering and the literal it analyzes to; `None` when one is
/// not a single literal (then the text is planned).
fn bind_values(params: &[JsonValue], format: &ParamFormat) -> Option<(Vec<String>, Vec<Literal>)> {
    let rendered: Vec<String> = params.iter().map(format).collect();
    let values = rendered
        .iter()
        .map(|r| template::param_literal(r))
        .collect::<Option<Vec<_>>>()?;
    Some((rendered, values))
}

fn signature(values: &[Literal]) -> String {
    values
        .iter()
        .map(|v| v.data_type().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

impl Template {
    /// The statement for `values`: a recent one with the same values, or a
    /// fresh binding. `None` when the values cannot be bound (then the text
    /// is planned).
    fn bind(
        &self,
        catalog: &Arc<dyn Catalog>,
        rendered: &[String],
        values: &[Literal],
    ) -> Option<Arc<Prepared>> {
        let value_key = rendered.join("\u{1}");
        if let Ok(recent) = self.recent.lock() {
            if let Some((_, statement)) = recent.iter().find(|(k, _)| *k == value_key) {
                return Some(statement.clone());
            }
        }
        let (analyzed, plan) = self.bind_parts(values).ok()?;
        let statement = Prepared::bound(analyzed, plan, catalog);
        if let Ok(mut recent) = self.recent.lock() {
            recent.push_front((value_key, statement.clone()));
            recent.truncate(RECENT_PER_TEMPLATE);
        }
        Some(statement)
    }

    /// The analyzed statement and optimized plan with `values` bound, the
    /// value-driven optimizer passes re-run.
    fn bind_parts(
        &self,
        values: &[Literal],
    ) -> Result<(AnalyzedStatement, Option<LogicalPlan>), String> {
        let mut analyzed = self.analyzed.clone();
        template::bind_statement(&mut analyzed, values)?;
        let plan = match &self.plan {
            Some(plan) => {
                let mut plan = plan.clone();
                template::bind_plan(&mut plan, values)?;
                Some(Optimizer::default().rebind(plan))
            }
            None => None,
        };
        Ok((analyzed, plan))
    }
}
