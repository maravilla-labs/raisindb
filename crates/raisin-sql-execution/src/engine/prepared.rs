//! Prepared statements: a statement's analysis and, for a query, its optimized
//! LOGICAL plan, cached per `(catalog, SQL text)` (plan Phase 13b / Phase 14
//! step 1).
//!
//! # Why
//!
//! Measured in a release build, parsing, semantic analysis, logical planning
//! and optimization were ~60 % of a SQL point lookup by path, and they are a pure
//! function of the SQL text and the catalog: the analyzer reads nothing else
//! (the function registry is static), constant folding folds only
//! deterministic functions, and nothing in the logical plan depends on time,
//! data, the caller or index state.
//!
//! # What is NOT cached, deliberately
//!
//! - **The physical plan.** It depends on index build state (spatial and
//!   compound availability), the compound-index definitions of the branch's
//!   NodeTypes, the read revision and schema statistics — all of which change
//!   without the SQL text changing. It is planned for every execution, from
//!   the cached logical plan.
//! - **Anything about the caller.** RLS, the auth context, the branch HEAD and
//!   the statement snapshot are applied at execution, after the cache.
//! - **Statements with uncorrelated subqueries.** Binding runs them and folds
//!   their DATA into the statement, so such a statement is never cached.
//! - **Anything but a query.** DML, DDL and the rest are analyzed each time.
//!
//! # Keys and invalidation
//!
//! The key holds the catalog's identity, and the entry a `Weak` to it, which
//! keeps the allocation (not the catalog) alive: an address in a live key
//! cannot be reused by a newer catalog. A schema change produces a NEW catalog
//! (`catalog_cache` rebuilds it on the workspace event, the TTL, and checkpoint
//! ingest), so it misses. The whole cache is also dropped through the derived
//! cache registry (checkpoint SST ingest emits no events).
//!
//! `RAISIN_SQL_PLAN_CACHE=0` turns the cache off (every statement is analyzed
//! and planned as before) — the rollback switch.

use raisin_error::Error;
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer, Catalog};
use raisin_sql::logical_plan::{LogicalPlan, PlanBuilder};
use raisin_sql::optimizer::Optimizer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

/// One analyzed statement and, for a query with a FROM clause, its optimized
/// logical plan.
pub(crate) struct Prepared {
    pub(crate) analyzed: AnalyzedStatement,
    pub(crate) plan: Option<LogicalPlan>,
    /// Its physical plan, once planned, with what it was planned from
    /// (`physical_cache.rs`). Only a statement with a `plan` uses it.
    pub(crate) physical: super::physical_cache::PhysicalSlot,
    /// Keeps the catalog's allocation reserved while this entry lives.
    _catalog: Option<Weak<dyn Catalog>>,
}

impl Prepared {
    /// A statement that is not cached (planned at execution, as before).
    fn uncached(analyzed: AnalyzedStatement) -> Arc<Self> {
        Arc::new(Self {
            analyzed,
            plan: None,
            physical: Default::default(),
            _catalog: None,
        })
    }

    /// A statement bound from a template (`prepared_params.rs`).
    pub(crate) fn bound(
        analyzed: AnalyzedStatement,
        plan: Option<LogicalPlan>,
        catalog: &Arc<dyn Catalog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            analyzed,
            plan,
            physical: Default::default(),
            _catalog: Some(Arc::downgrade(catalog)),
        })
    }
}

/// `(catalog identity, single statement vs batch, SQL text)`.
type Key = (usize, bool, String);

/// SQL longer than this is never cached (a generated bulk statement).
pub(super) const MAX_SQL_LEN: usize = 8 * 1024;
/// Rough heap weight of one entry beyond its SQL text: an analyzed `SELECT *`
/// and its logical plan carry every column of the table, twice. With the
/// 64 MiB budget below that is about 2,000 statements.
pub(super) const ENTRY_WEIGHT: u32 = 32 * 1024;

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("RAISIN_SQL_PLAN_CACHE")
            .map(|v| {
                !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
    })
}

static CACHE: OnceLock<moka::sync::Cache<Key, Arc<Prepared>>> = OnceLock::new();

fn cache() -> &'static moka::sync::Cache<Key, Arc<Prepared>> {
    CACHE.get_or_init(|| {
        // Checkpoint ingest replaces whole column families without events.
        raisin_core::register_invalidator(invalidate_plan_cache);
        moka::sync::Cache::builder()
            .max_capacity(64 * 1024 * 1024)
            .weigher(|key: &Key, _: &Arc<Prepared>| {
                ENTRY_WEIGHT.saturating_add(u32::try_from(key.2.len() * 4).unwrap_or(u32::MAX))
            })
            .build()
    })
}

pub(super) static HITS: AtomicU64 = AtomicU64::new(0);
pub(super) static MISSES: AtomicU64 = AtomicU64::new(0);

/// Drop every cached statement and template.
pub fn invalidate_plan_cache() {
    if let Some(cache) = CACHE.get() {
        cache.invalidate_all();
    }
    super::prepared_params::invalidate_templates();
}

/// `(hits, misses)` since the process started — for tests and diagnostics.
#[doc(hidden)]
pub fn plan_cache_stats() -> (u64, u64) {
    (HITS.load(Ordering::Relaxed), MISSES.load(Ordering::Relaxed))
}

/// Whether `sql` (as one statement, or as a batch) is cached against
/// `catalog` — for tests; it does not count as a lookup.
#[doc(hidden)]
pub fn plan_cache_contains(catalog: &Arc<dyn Catalog>, sql: &str, batch: bool) -> bool {
    CACHE
        .get()
        .is_some_and(|cache| cache.contains_key(&key(catalog, batch, sql)))
}

fn key(catalog: &Arc<dyn Catalog>, batch: bool, sql: &str) -> Key {
    (
        Arc::as_ptr(catalog) as *const () as usize,
        batch,
        sql.to_string(),
    )
}

fn lookup(key: &Key) -> Option<Arc<Prepared>> {
    let found = cache().get(key);
    let counter = if found.is_some() { &HITS } else { &MISSES };
    counter.fetch_add(1, Ordering::Relaxed);
    found
}

/// Cache `analyzed` when it is a query that binds nothing; returns the
/// prepared statement either way.
fn prepare_one(
    catalog: &Arc<dyn Catalog>,
    analyzed: AnalyzedStatement,
    cache_key: Option<Key>,
) -> Result<Arc<Prepared>, Error> {
    let cacheable = matches!(analyzed, AnalyzedStatement::Query(_))
        && !super::subquery_bind::statement_needs_binding(&analyzed);
    let (Some(cache_key), true) = (cache_key, cacheable) else {
        return Ok(Prepared::uncached(analyzed));
    };
    let plan = match &analyzed {
        AnalyzedStatement::Query(q) if !q.from.is_empty() => {
            Some(logical_plan(catalog, &analyzed)?)
        }
        _ => None,
    };
    let prepared = Arc::new(Prepared {
        analyzed,
        plan,
        physical: Default::default(),
        _catalog: Some(Arc::downgrade(catalog)),
    });
    cache().insert(cache_key, prepared.clone());
    Ok(prepared)
}

/// The optimized logical plan of an analyzed query.
pub(crate) fn logical_plan(
    catalog: &Arc<dyn Catalog>,
    analyzed: &AnalyzedStatement,
) -> Result<LogicalPlan, Error> {
    let plan = PlanBuilder::new(catalog.as_ref())
        .build(analyzed)
        .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;
    Ok(Optimizer::default().optimize(plan))
}

fn cache_key_for(catalog: &Arc<dyn Catalog>, batch: bool, sql: &str) -> Option<Key> {
    (enabled() && sql.len() <= MAX_SQL_LEN).then(|| key(catalog, batch, sql))
}

/// Analyze ONE statement (`QueryEngine::execute`), from the cache when it can.
pub(crate) fn prepare_statement(
    catalog: &Arc<dyn Catalog>,
    sql: &str,
) -> Result<Arc<Prepared>, Error> {
    prepare_statement_traced(catalog, sql).map(|(prepared, _)| prepared)
}

/// [`prepare_statement`], also saying whether THIS call was answered from
/// the cache (residency alone is no proof under eviction pressure).
pub(crate) fn prepare_statement_traced(
    catalog: &Arc<dyn Catalog>,
    sql: &str,
) -> Result<(Arc<Prepared>, bool), Error> {
    let cache_key = cache_key_for(catalog, false, sql);
    if let Some(hit) = cache_key.as_ref().and_then(lookup) {
        return Ok((hit, true));
    }
    let analyzed = Analyzer::with_catalog_arc(catalog.clone())
        .analyze(sql)
        .map_err(|e| Error::Validation(format!("Analysis error: {}", e)))?;
    Ok((prepare_one(catalog, analyzed, cache_key)?, false))
}

/// Analyze a batch (`QueryEngine::execute_batch`). A batch of ONE statement
/// is cached like a single statement; a longer batch never is.
pub(crate) fn prepare_batch(
    catalog: &Arc<dyn Catalog>,
    sql: &str,
) -> Result<Vec<Arc<Prepared>>, Error> {
    prepare_batch_traced(catalog, sql).map(|(statements, _)| statements)
}

/// [`prepare_batch`], also saying whether THIS call was answered from the
/// cache.
pub(crate) fn prepare_batch_traced(
    catalog: &Arc<dyn Catalog>,
    sql: &str,
) -> Result<(Vec<Arc<Prepared>>, bool), Error> {
    let cache_key = cache_key_for(catalog, true, sql);
    if let Some(hit) = cache_key.as_ref().and_then(lookup) {
        return Ok((vec![hit], true));
    }
    let mut statements = Analyzer::with_catalog_arc(catalog.clone())
        .analyze_batch(sql)
        .map_err(|e| Error::Validation(format!("Batch analysis error: {}", e)))?;
    if statements.len() == 1 {
        let only = statements.pop().expect("one statement");
        return Ok((vec![prepare_one(catalog, only, cache_key)?], false));
    }
    Ok((
        statements.into_iter().map(Prepared::uncached).collect(),
        false,
    ))
}
