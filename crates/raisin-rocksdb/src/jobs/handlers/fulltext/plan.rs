//! Shape-driven full-text indexing plan resolution.
//!
//! Resolves, per node, which field values to index and which shape identities to
//! emit, driven by the node's NodeType / Archetype / nested ElementType configs.
//! Resolved per-*type* decisions are cached (keyed by scope + type name) so the
//! hot indexing path performs storage lookups only on a cache miss — steady state
//! is an `Arc` clone per distinct type.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use raisin_core::services::archetype_resolver::ArchetypeResolver;
use raisin_core::services::element_type_resolver::ElementTypeResolver;
use raisin_core::services::indexing_policy::IndexingPolicy;
use raisin_core::services::node_type_resolver::NodeTypeResolver;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::IndexType;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::element::field_types::FieldSchema;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::{ElementFieldPlan, NodeIndexPlan};

use crate::RocksDBStorage;

const CACHE_CAPACITY: u64 = 10_000;
const CACHE_TTI_SECS: u64 = 300;
/// Hard upper bound on staleness (NOT reset on access), so type-definition edits
/// propagate even for continuously-hot types. time_to_idle alone never expires a
/// type that is indexed without pause (e.g. during a bulk import).
const CACHE_TTL_SECS: u64 = 300;

/// Cached, resolved decision for a single NodeType.
#[derive(Clone)]
struct NodeTypePlan {
    /// Whether the type opts into indexing for the requested `index_type` at all
    /// (`indexable` + `index_types` contains it). Ignored for `legacy` entries.
    indexable: bool,
    /// The node type could not be resolved — fall back to indexing all top-level
    /// String properties (legacy behavior for untyped/unknown nodes).
    legacy: bool,
    /// Top-level property names whose value is configured for the requested index.
    top_level_props: Vec<String>,
}

/// Per-type plan cache shared across nodes for a handler's lifetime.
pub struct IndexPlanCache {
    node_types: Cache<String, Arc<NodeTypePlan>>,
    archetypes: Cache<String, Arc<Vec<String>>>,
    element_types: Cache<String, Arc<ElementFieldPlan>>,
}

impl Default for IndexPlanCache {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexPlanCache {
    pub fn new() -> Self {
        Self {
            node_types: Cache::builder()
                .max_capacity(CACHE_CAPACITY)
                .time_to_idle(Duration::from_secs(CACHE_TTI_SECS))
                .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
                .build(),
            archetypes: Cache::builder()
                .max_capacity(CACHE_CAPACITY)
                .time_to_idle(Duration::from_secs(CACHE_TTI_SECS))
                .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
                .build(),
            element_types: Cache::builder()
                .max_capacity(CACHE_CAPACITY)
                .time_to_idle(Duration::from_secs(CACHE_TTI_SECS))
                .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
                .build(),
        }
    }
}

/// Cache key for a resolved type decision. The `index_type` discriminant keeps
/// Fulltext and Vector plans for the same type from colliding in the cache.
fn scope_key(tenant: &str, repo: &str, branch: &str, name: &str, index_type: &IndexType) -> String {
    format!(
        "{:?}\0{}\0{}\0{}\0{}",
        index_type, tenant, repo, branch, name
    )
}

/// Whether a resolved field opts its value into the requested index type.
fn field_has_index(field: &FieldSchema, index_type: &IndexType) -> bool {
    field
        .base()
        .index
        .as_ref()
        .map(|idx| idx.contains(index_type))
        .unwrap_or(false)
}

/// Resolve the indexing plan for `node` for the requested `index_type` (e.g.
/// `Fulltext` or `Vector`), or `None` if the node's type explicitly opts out of
/// that index (so it should not be indexed).
pub async fn resolve_index_plan(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    node: &Node,
    cache: &IndexPlanCache,
    index_type: &IndexType,
) -> Result<Option<NodeIndexPlan>> {
    // 1. NodeType decision (cached per type).
    let nt_key = scope_key(tenant, repo, branch, &node.node_type, index_type);
    let nt_plan = match cache.node_types.get(&nt_key) {
        Some(p) => p,
        None => {
            let resolver = NodeTypeResolver::new(
                Arc::clone(storage),
                tenant.to_string(),
                repo.to_string(),
                branch.to_string(),
            );
            let plan = match resolver.resolve(&node.node_type).await {
                Ok(resolved) => NodeTypePlan {
                    indexable: IndexingPolicy::should_index_node(&resolved, index_type),
                    legacy: false,
                    top_level_props: IndexingPolicy::properties_to_index(&resolved, index_type),
                },
                Err(e) => {
                    tracing::debug!(
                        node_type = %node.node_type,
                        error = %e,
                        "NodeType unresolved; falling back to legacy index-all-strings"
                    );
                    NodeTypePlan {
                        indexable: true,
                        legacy: true,
                        top_level_props: Vec::new(),
                    }
                }
            };
            let arc = Arc::new(plan);
            cache.node_types.insert(nt_key, Arc::clone(&arc));
            arc
        }
    };

    // Respect an explicit opt-out (resolved type that disables this index).
    if !nt_plan.legacy && !nt_plan.indexable {
        return Ok(None);
    }

    // 2. Top-level property selection. Legacy => index all top-level Strings.
    let (top_level_props, legacy_index_all_strings) = if nt_plan.legacy {
        (None, true)
    } else {
        let mut props = nt_plan.top_level_props.clone();

        // Fold in archetype fields configured for the requested index.
        if let Some(archetype) = &node.archetype {
            let a_key = scope_key(tenant, repo, branch, archetype, index_type);
            let arch_props = match cache.archetypes.get(&a_key) {
                Some(p) => p,
                None => {
                    let resolver = ArchetypeResolver::new(
                        Arc::clone(storage),
                        tenant.to_string(),
                        repo.to_string(),
                        branch.to_string(),
                    );
                    let fields = match resolver.resolve(archetype).await {
                        Ok(resolved) => resolved
                            .resolved_fields
                            .iter()
                            .filter(|f| field_has_index(f, index_type))
                            .map(|f| f.base().name.clone())
                            .collect(),
                        Err(e) => {
                            tracing::debug!(archetype = %archetype, error = %e, "Archetype unresolved");
                            Vec::new()
                        }
                    };
                    let arc = Arc::new(fields);
                    cache.archetypes.insert(a_key, Arc::clone(&arc));
                    arc
                }
            };
            props.extend(arch_props.iter().cloned());
        }

        (Some(props), false)
    };

    // 3. Nested ElementType field plans (cached per element type).
    let mut element_plans = HashMap::new();
    for element_type in collect_element_types(&node.properties) {
        let e_key = scope_key(tenant, repo, branch, &element_type, index_type);
        let plan = match cache.element_types.get(&e_key) {
            Some(p) => p,
            None => {
                let resolver = ElementTypeResolver::new(
                    Arc::clone(storage),
                    tenant.to_string(),
                    repo.to_string(),
                    branch.to_string(),
                );
                let plan = match resolver.resolve(&element_type).await {
                    Ok(resolved) => ElementFieldPlan {
                        include_fields: resolved
                            .resolved_fields
                            .iter()
                            .filter(|f| field_has_index(f, index_type))
                            .map(|f| f.base().name.clone())
                            .collect(),
                    },
                    Err(e) => {
                        tracing::debug!(element_type = %element_type, error = %e, "ElementType unresolved");
                        ElementFieldPlan::default()
                    }
                };
                let arc = Arc::new(plan);
                cache.element_types.insert(e_key, Arc::clone(&arc));
                arc
            }
        };
        element_plans.insert(element_type, (*plan).clone());
    }

    Ok(Some(NodeIndexPlan {
        node_type: node.node_type.clone(),
        archetype: node.archetype.clone(),
        top_level_props,
        element_plans,
        legacy_index_all_strings,
    }))
}

/// Collects the distinct `element_type` names of every nested Element/Composite
/// block in a node's properties (iterative; overflow-safe for deep nesting).
/// Every ElementType name that appears anywhere in a node's property tree.
///
/// `pub(crate)` because cross-branch promotion needs exactly this set to know
/// which ElementType definitions to carry onto the target branch
/// (`repositories/nodes/queries/copy/cross_branch/schema.rs`). It is shared
/// rather than reimplemented there on purpose: a promotion that collected a
/// DIFFERENT set from the one the planner resolves would carry the wrong
/// definitions, and the two would drift silently — a node whose blocks are
/// nested one level deeper than the copier looks would publish with no schema
/// and fall back to legacy indexing, with nothing logged.
pub(crate) fn collect_element_types(
    properties: &HashMap<String, PropertyValue>,
) -> HashSet<String> {
    let mut found = HashSet::new();
    let mut stack: Vec<&PropertyValue> = properties.values().collect();

    while let Some(value) = stack.pop() {
        match value {
            PropertyValue::Element(block) => {
                found.insert(block.element_type.clone());
                stack.extend(block.content.values());
            }
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    found.insert(block.element_type.clone());
                    stack.extend(block.content.values());
                }
            }
            PropertyValue::Array(arr) => stack.extend(arr.iter()),
            PropertyValue::Object(obj) => stack.extend(obj.values()),
            _ => {}
        }
    }

    found
}
