//! The physical plan of a prepared statement, kept beside it (plan Phase 13d).
//!
//! A physical plan is a pure function of the optimized logical plan and of
//! what the planner reads besides it:
//!
//! | input | how a cached plan is checked against it |
//! |---|---|
//! | tenant, repo, the engine's branch, the FROM workspace | equal |
//! | the branch's compound-index definitions the planner was handed | equal |
//! | schema statistics (only read for a `node_type`/`archetype` equality) | equal |
//! | the statement's `__revision` pin | part of the statement |
//! | every spatial / compound AVAILABILITY answer the planner asked for | asked again, equal |
//!
//! The last row is what makes the cache safe against an index build, a
//! rebuild or a declaration change: the planner's questions are RECORDED while
//! it plans ([`RecordingCatalog`]), and a reuse asks each of them again —
//! fail closed, through the same state stores — and replans on any different
//! answer. A plan that never asked (a path or id lookup with no compound
//! index in reach) is reused after the equality checks alone.
//!
//! NOT in a physical plan, and therefore not cached: the branch HEAD (the
//! execution context reads it per execution), the caller (RLS and auth are in
//! the context), the statement snapshot, the RESOLVE memo. Plans whose
//! statement is not cached (DML, a statement with subqueries, a never-seen
//! text the cache did not admit) are planned per execution as before.

use crate::physical_plan::catalog::SpatialAvailability;
use crate::physical_plan::operators::PhysicalPlan;
use crate::physical_plan::planner::SchemaStats;
use crate::physical_plan::IndexCatalog;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::CompoundAvailability;
use std::sync::{Arc, Mutex};

/// One availability question the planner asked, and the answer it got.
#[derive(Debug, Clone, PartialEq)]
enum Probe {
    Spatial {
        scope: [String; 4],
        property: String,
        answer: SpatialAvailability,
    },
    Compound {
        scope: [String; 4],
        definition: CompoundIndexDefinition,
        answer: CompoundAvailability,
    },
}

/// Everything besides the logical plan a physical plan was planned from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannerInputs {
    pub(crate) tenant: String,
    pub(crate) repo: String,
    pub(crate) branch: String,
    pub(crate) workspace: String,
    pub(crate) compound: Option<Vec<CompoundIndexDefinition>>,
    pub(crate) schema_stats: Option<SchemaStats>,
}

/// A cached physical plan and what it was planned from.
pub(crate) struct CachedPhysical {
    inputs: PlannerInputs,
    probes: Vec<Probe>,
    plan: Arc<PhysicalPlan>,
}

impl CachedPhysical {
    /// The plan, when `inputs` are the ones it was planned with and every
    /// availability answer it depended on is still the same.
    pub(crate) fn reuse(
        &self,
        inputs: &PlannerInputs,
        catalog: &dyn IndexCatalog,
    ) -> Option<Arc<PhysicalPlan>> {
        if &self.inputs != inputs {
            return None;
        }
        let unchanged = self.probes.iter().all(|probe| match probe {
            Probe::Spatial {
                scope: [t, r, b, w],
                property,
                answer,
            } => &catalog.spatial_index_availability(t, r, b, w, property) == answer,
            Probe::Compound {
                scope: [t, r, b, w],
                definition,
                answer,
            } => &catalog.compound_index_availability(t, r, b, w, definition) == answer,
        });
        unchanged.then(|| self.plan.clone())
    }
}

/// The slot a prepared statement keeps its physical plan in. One plan per
/// statement: a statement planned again with other inputs (another branch,
/// a changed index) replaces it.
#[derive(Default)]
pub(crate) struct PhysicalSlot(Mutex<Option<Arc<CachedPhysical>>>);

impl PhysicalSlot {
    pub(crate) fn get(&self) -> Option<Arc<CachedPhysical>> {
        self.0.lock().ok().and_then(|slot| slot.clone())
    }

    pub(crate) fn put(
        &self,
        inputs: PlannerInputs,
        recorder: &RecordingCatalog,
        plan: Arc<PhysicalPlan>,
    ) {
        let entry = Arc::new(CachedPhysical {
            inputs,
            probes: recorder.probes(),
            plan,
        });
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(entry);
        }
    }
}

/// An index catalog that remembers every availability question asked of it.
pub(crate) struct RecordingCatalog {
    inner: Arc<dyn IndexCatalog>,
    probes: Mutex<Vec<Probe>>,
}

impl RecordingCatalog {
    pub(crate) fn new(inner: Arc<dyn IndexCatalog>) -> Self {
        Self {
            inner,
            probes: Mutex::new(Vec::new()),
        }
    }

    fn probes(&self) -> Vec<Probe> {
        self.probes.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn record(&self, probe: Probe) {
        if let Ok(mut probes) = self.probes.lock() {
            probes.push(probe);
        }
    }
}

fn scope(t: &str, r: &str, b: &str, w: &str) -> [String; 4] {
    [t.to_string(), r.to_string(), b.to_string(), w.to_string()]
}

impl IndexCatalog for RecordingCatalog {
    fn has_path_index(&self) -> bool {
        self.inner.has_path_index()
    }

    fn has_property_index(&self) -> bool {
        self.inner.has_property_index()
    }

    fn has_fulltext_index(&self) -> bool {
        self.inner.has_fulltext_index()
    }

    fn spatial_index_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> SpatialAvailability {
        let answer = self
            .inner
            .spatial_index_availability(tenant_id, repo_id, branch, workspace, property);
        self.record(Probe::Spatial {
            scope: scope(tenant_id, repo_id, branch, workspace),
            property: property.to_string(),
            answer: answer.clone(),
        });
        answer
    }

    fn has_compound_index(&self) -> bool {
        self.inner.has_compound_index()
    }

    fn compound_index_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability {
        let answer = self
            .inner
            .compound_index_availability(tenant_id, repo_id, branch, workspace, definition);
        self.record(Probe::Compound {
            scope: scope(tenant_id, repo_id, branch, workspace),
            definition: definition.clone(),
            answer: answer.clone(),
        });
        answer
    }

    fn find_compound_index(
        &self,
        node_type: &str,
        equality_columns: &[&str],
        order_column: &str,
        ascending: bool,
    ) -> Option<(String, usize)> {
        self.inner
            .find_compound_index(node_type, equality_columns, order_column, ascending)
    }

    fn available_indexes(&self) -> Vec<String> {
        self.inner.available_indexes()
    }
}
