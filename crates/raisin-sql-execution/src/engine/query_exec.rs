//! The ONE query path: physical planning and execution of an analyzed SELECT.
//!
//! `QueryEngine::execute` (functions, tests) and `execute_batch` (HTTP, WS,
//! pgwire) each carried their own copy of this body — logical plan, optimizer,
//! index catalog, compound indexes, schema statistics, HEAD read, execution
//! context — and only the batch copy knew a SELECT without FROM. Both now
//! call [`QueryEngine::plan_query`], which takes the cached logical plan when
//! the statement was prepared from the cache (`prepared.rs`) and plans
//! physically every time.

use super::physical_cache::{PlannerInputs, RecordingCatalog};
use super::prepared::Prepared;
use super::prepared_params::ParamOutcome;
use super::{helpers, prepared, QueryEngine};
use crate::physical_plan::executor::{execute_plan, ExecutionContext, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use crate::physical_plan::planner::PhysicalPlanner;
use crate::physical_plan::IndexCatalog;
use raisin_error::Error;
use raisin_sql::analyzer::AnalyzedStatement;
use raisin_sql::logical_plan::LogicalPlan;
use raisin_storage::{BranchRepository, Storage};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute an analyzed SELECT. `prepared` is the statement it came from
    /// when it was prepared (cached): its optimized logical plan and its
    /// physical-plan slot are used; otherwise both are planned here.
    pub(crate) async fn execute_query(
        &self,
        analyzed: &AnalyzedStatement,
        prepared: Option<&Prepared>,
    ) -> Result<RowStream, Error> {
        // A SELECT without FROM never reaches the planner (`handlers/scalar.rs`).
        if let Some(scalar) = self.execute_scalar_if_no_from(analyzed).await {
            return scalar;
        }
        let (physical_plan, ctx) = self.plan_query(analyzed, prepared).await?;
        execute_plan(&physical_plan, &ctx).await
    }

    /// The physical plan of an analyzed SELECT with a FROM clause, and the
    /// execution context it runs in.
    ///
    /// Everything that is NOT a function of the SQL text is decided here, per
    /// execution: index availability, the branch's compound indexes, schema
    /// statistics, the read revision (HEAD when the query names none), the
    /// caller's auth context. A prepared statement's physical plan is reused
    /// only when all of them are what it was planned with
    /// (`physical_cache.rs`).
    pub(crate) async fn plan_query(
        &self,
        analyzed: &AnalyzedStatement,
        prepared: Option<&Prepared>,
    ) -> Result<(Arc<PhysicalPlan>, ExecutionContext<S>), Error> {
        let physical_plan = self.physical_plan_cached(analyzed, prepared).await?;
        let query = match analyzed {
            AnalyzedStatement::Query(q) => Some(q),
            _ => None,
        };
        let (max_revision, branch_override, locales) = match query {
            Some(q) => (q.max_revision, q.branch_override.clone(), q.locales.clone()),
            None => (None, None, Vec::new()),
        };
        let branch = branch_override.unwrap_or_else(|| self.branch.clone());
        let max_revision = match max_revision {
            Some(revision) => Some(revision),
            None => Some(
                self.storage
                    .branches()
                    .get_branch(&self.tenant_id, &self.repo_id, &branch)
                    .await?
                    .map(|b| b.head)
                    .unwrap_or_else(|| raisin_hlc::HLC::new(0, 0)),
            ),
        };
        let ctx =
            self.build_execution_context(branch, query_workspace(analyzed), max_revision, locales);
        Ok((physical_plan, ctx))
    }

    /// The physical plan of an analyzed SELECT — THE planner call behind
    /// execution and EXPLAIN, so EXPLAIN shows the plan a query runs.
    /// `plan` is the optimized logical plan when the caller has one.
    pub(crate) async fn physical_plan_for(
        &self,
        analyzed: &AnalyzedStatement,
        plan: Option<&LogicalPlan>,
    ) -> Result<PhysicalPlan, Error> {
        let inputs = self.planner_inputs(analyzed).await;
        let catalog = self.index_catalog();
        let built;
        let optimized = match plan {
            Some(plan) => plan,
            None => {
                built = prepared::logical_plan(&self.catalog, analyzed)?;
                &built
            }
        };
        self.plan_physical(analyzed, optimized, &inputs, catalog)
    }

    /// [`Self::physical_plan_for`] through the prepared statement's physical
    /// slot: reused when it was planned from the same inputs and every
    /// availability answer it asked for is unchanged, planned (and kept)
    /// otherwise.
    async fn physical_plan_cached(
        &self,
        analyzed: &AnalyzedStatement,
        prepared: Option<&Prepared>,
    ) -> Result<Arc<PhysicalPlan>, Error> {
        self.physical_plan_traced(analyzed, prepared)
            .await
            .map(|(plan, _)| plan)
    }

    /// [`Self::physical_plan_cached`], also saying whether the plan was
    /// reused from the statement's slot.
    async fn physical_plan_traced(
        &self,
        analyzed: &AnalyzedStatement,
        prepared: Option<&Prepared>,
    ) -> Result<(Arc<PhysicalPlan>, bool), Error> {
        let Some((prepared, logical)) = prepared.and_then(|p| p.plan.as_ref().map(|l| (p, l)))
        else {
            return Ok((
                Arc::new(self.physical_plan_for(analyzed, None).await?),
                false,
            ));
        };
        let inputs = self.planner_inputs(analyzed).await;
        let catalog = self.index_catalog();
        if let Some(cached) = prepared.physical.get() {
            if let Some(plan) = cached.reuse(&inputs, catalog.as_ref()) {
                PHYSICAL_HITS.fetch_add(1, Ordering::Relaxed);
                return Ok((plan, true));
            }
        }
        let recorder = Arc::new(RecordingCatalog::new(catalog));
        let plan = Arc::new(self.plan_physical(analyzed, logical, &inputs, recorder.clone())?);
        prepared.physical.put(inputs, &recorder, plan.clone());
        Ok((plan, false))
    }

    /// The index catalog with the spatial and compound index build states.
    /// Without them it answers `NotBuilt` for every index and the planner
    /// declines it — correct, never fast; with them, a declared-but-unbuilt
    /// index is distinguishable from a usable one.
    fn index_catalog(&self) -> Arc<dyn IndexCatalog> {
        Arc::new(
            crate::physical_plan::catalog::RocksDBIndexCatalog::new()
                .with_optional_spatial_state(self.storage.spatial_state())
                .with_optional_compound_state(self.storage.compound_state()),
        )
    }

    /// Everything the physical planner reads besides the logical plan and the
    /// index catalog.
    async fn planner_inputs(&self, analyzed: &AnalyzedStatement) -> PlannerInputs {
        // Compound indexes for this branch: the named NodeType's when the
        // WHERE clause pins one, otherwise every index on the branch (a
        // hierarchy query is usually written without `node_type =`).
        let compound = match helpers::extract_node_type_from_analyzed(analyzed) {
            Some(node_type_name) => {
                helpers::load_compound_indexes(
                    &*self.storage,
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &node_type_name,
                )
                .await
            }
            None => {
                helpers::load_all_compound_indexes(
                    &*self.storage,
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                )
                .await
            }
        };
        // Gated inside on the WHERE clause containing a node_type/archetype
        // equality — see `schema_stats_for`.
        let schema_stats = self
            .schema_stats_for(&self.branch, helpers::analyzed_selection(analyzed))
            .await;
        PlannerInputs {
            tenant: self.tenant_id.clone(),
            repo: self.repo_id.clone(),
            branch: self.branch.clone(),
            workspace: query_workspace(analyzed),
            compound,
            schema_stats,
        }
    }

    fn plan_physical(
        &self,
        analyzed: &AnalyzedStatement,
        optimized: &LogicalPlan,
        inputs: &PlannerInputs,
        catalog: Arc<dyn IndexCatalog>,
    ) -> Result<PhysicalPlan, Error> {
        let mut physical_planner = PhysicalPlanner::with_catalog(
            inputs.tenant.clone(),
            inputs.repo.clone(),
            inputs.branch.clone(),
            inputs.workspace.clone(),
            catalog,
        );
        if let Some(indexes) = &inputs.compound {
            physical_planner.set_compound_indexes(indexes.clone());
        }
        // A `__revision = N` read: compound indexes answer only at or above
        // their build's history floor.
        if let AnalyzedStatement::Query(q) = analyzed {
            physical_planner.set_read_revision(q.max_revision);
        }
        if let Some(stats) = &inputs.schema_stats {
            physical_planner.set_schema_statistics(stats.clone());
        }
        physical_planner.plan(optimized)
    }

    /// The physical plan `sql` runs with, planned through the prepared
    /// statement exactly as [`execute`](Self::execute) plans it, and whether
    /// the statement came from the prepared-statement cache — for tests and
    /// diagnostics.
    #[doc(hidden)]
    pub async fn prepared_physical_plan(&self, sql: &str) -> Result<(String, bool), Error> {
        let (prepared, cached) = prepared::prepare_statement_traced(&self.catalog, sql)?;
        let plan = self
            .physical_plan_cached(&prepared.analyzed, Some(&prepared))
            .await?;
        Ok((plan.explain(), cached))
    }

    /// The physical plan `sql` with `params` runs with (planned as
    /// [`execute_with_params`](Self::execute_with_params) plans it), how its
    /// statement was prepared, and whether the physical plan was reused —
    /// for tests and diagnostics.
    #[doc(hidden)]
    pub async fn prepared_physical_plan_with_params(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> Result<(String, ParamOutcome, bool), Error> {
        let bound = super::prepared_params::prepare_with_params(
            &self.catalog,
            false,
            sql,
            params,
            &raisin_sql::format_param_value,
        )?;
        let prepared = bound
            .statements
            .first()
            .ok_or_else(|| Error::Validation("No statement".to_string()))?;
        let (plan, reused) = self
            .physical_plan_traced(&prepared.analyzed, Some(prepared))
            .await?;
        Ok((plan.explain(), bound.outcome, reused))
    }
}

/// Physical plans answered from a prepared statement's slot — for tests.
static PHYSICAL_HITS: AtomicU64 = AtomicU64::new(0);

/// How many executions reused a cached physical plan since the process
/// started — for tests and diagnostics.
#[doc(hidden)]
pub fn physical_plan_cache_hits() -> u64 {
    PHYSICAL_HITS.load(Ordering::Relaxed)
}

/// The workspace a query reads (its first FROM table), `default` otherwise.
fn query_workspace(analyzed: &AnalyzedStatement) -> String {
    match analyzed {
        AnalyzedStatement::Query(q) => q.from.first().and_then(|t| t.workspace.clone()),
        _ => None,
    }
    .unwrap_or_else(|| "default".to_string())
}
