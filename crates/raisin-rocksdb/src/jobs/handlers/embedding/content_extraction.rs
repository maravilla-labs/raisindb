//! Content extraction for embedding generation.
//!
//! Extracts embeddable text content from nodes driven by a shape-aware
//! [`NodeIndexPlan`] resolved for [`IndexType::Vector`]. Unlike full-text
//! indexing, the embedding path emits *values only* — no shape-type identity
//! tokens — so semantic embeddings stay free of `ns:TypeName` noise.

use crate::jobs::handlers::fulltext::{resolve_index_plan, IndexPlanCache};
use crate::RocksDBStorage;
use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_error::Result;
use raisin_models::nodes::properties::schema::IndexType;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::NodeIndexPlan;
use raisin_storage::jobs::JobContext;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Extract embeddable content from a node, driven by its vector indexing plan.
///
/// The shape-driven approach mirrors full-text indexing: it embeds top-level
/// fields marked `Vector`, archetype fields marked `Vector`, and nested element
/// fields marked `Vector` (gated per element type). It indexes VALUES ONLY — it
/// never emits shape-type identity tokens.
///
/// Name/path inclusion follows the tenant config. If the node's type opts out of
/// vector indexing (plan is `None`), only name/path are returned.
pub async fn extract_embeddable_content(
    node: &Node,
    config: &TenantEmbeddingConfig,
    storage: Arc<RocksDBStorage>,
    context: &JobContext,
    cache: &IndexPlanCache,
) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Include node name (from global config)
    if config.include_name {
        parts.push(node.name.clone());
    }

    // 2. Include node path (from global config)
    if config.include_path {
        parts.push(node.path.clone());
    }

    // 3. Resolve the shape-driven VECTOR plan (cached per type).
    let plan = resolve_index_plan(
        &storage,
        &context.tenant_id,
        &context.repo_id,
        &context.branch,
        node,
        cache,
        &IndexType::Vector,
    )
    .await?;

    let Some(plan) = plan else {
        // Type opts out of vector indexing -> name/path only.
        tracing::debug!(
            node_type = %node.node_type,
            "Node type is not configured for vector indexing, returning name+path only"
        );
        return Ok(parts.join("\n"));
    };

    // 4. Walk properties using the plan, collecting VALUES ONLY.
    let values = collect_plan_values(&plan, &node.properties);
    if !values.is_empty() {
        parts.push(values);
    }

    Ok(parts.join("\n"))
}

/// Pure, storage-free walk that collects embeddable VALUE text per the plan.
///
/// Top-level property values are included iff their name is in
/// `plan.top_level_props` (when `Some`), else — for legacy/unresolved types —
/// all top-level `String`s are included. Nested Element/Composite fields are
/// gated by `plan.element_plans[element_type].include_fields`, independent of any
/// ancestor's inclusion. An unknown element type contributes no values. Arrays
/// and Objects inherit their parent's include flag. NO shape identity tokens are
/// emitted. Returns the joined text (empty string if nothing was collected).
pub(crate) fn collect_plan_values(
    plan: &NodeIndexPlan,
    properties: &HashMap<String, PropertyValue>,
) -> String {
    let mut out: Vec<String> = Vec::new();

    let selected: Option<std::collections::HashSet<&str>> = plan
        .top_level_props
        .as_ref()
        .map(|p| p.iter().map(|s| s.as_str()).collect());

    for (name, value) in properties {
        let index_value = match &selected {
            Some(set) => set.contains(name.as_str()),
            None => plan.legacy_index_all_strings && matches!(value, PropertyValue::String(_)),
        };
        walk_values(value, index_value, plan, &mut out);
    }

    out.join("\n")
}

/// Iterative, overflow-safe walk emitting scalar VALUES (gated). Element/Composite
/// sub-fields are gated by the element type's own `include_fields`, independent of
/// the incoming `index` flag. No identity tokens are emitted.
fn walk_values(
    value: &PropertyValue,
    index_value: bool,
    plan: &NodeIndexPlan,
    out: &mut Vec<String>,
) {
    let mut stack: Vec<(&PropertyValue, bool)> = vec![(value, index_value)];

    while let Some((current, idx)) = stack.pop() {
        match current {
            PropertyValue::String(s) => {
                if idx && !s.is_empty() {
                    out.push(s.clone());
                }
            }
            PropertyValue::Integer(n) => {
                if idx {
                    out.push(n.to_string());
                }
            }
            PropertyValue::Float(f) => {
                if idx {
                    out.push(f.to_string());
                }
            }
            PropertyValue::Boolean(b) => {
                if idx {
                    out.push(b.to_string());
                }
            }
            PropertyValue::Date(d) => {
                if idx {
                    out.push(d.to_string());
                }
            }
            PropertyValue::Decimal(dec) => {
                if idx {
                    out.push(dec.to_string());
                }
            }
            PropertyValue::Url(u) => {
                if idx {
                    if !u.url.is_empty() {
                        out.push(u.url.clone());
                    }
                    for extra in [&u.label, &u.title, &u.description].into_iter().flatten() {
                        if !extra.is_empty() {
                            out.push(extra.clone());
                        }
                    }
                }
            }
            PropertyValue::Array(arr) => {
                for item in arr.iter().rev() {
                    stack.push((item, idx));
                }
            }
            PropertyValue::Object(obj) => {
                for v in obj.values() {
                    stack.push((v, idx));
                }
            }
            PropertyValue::Element(block) => {
                push_element_content(&block.element_type, &block.content, plan, &mut stack);
            }
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    push_element_content(&block.element_type, &block.content, plan, &mut stack);
                }
            }
            PropertyValue::Null
            | PropertyValue::Reference(_)
            | PropertyValue::Resource(_)
            | PropertyValue::Vector(_)
            | PropertyValue::Geometry(_) => {}
        }
    }
}

/// Pushes an element block's content fields onto the walk stack, gating each
/// field by the element type's `include_fields` plan. An unknown element type
/// contributes none of its values.
fn push_element_content<'a>(
    element_type: &str,
    block_content: &'a HashMap<String, PropertyValue>,
    plan: &NodeIndexPlan,
    stack: &mut Vec<(&'a PropertyValue, bool)>,
) {
    let element_plan = plan.element_plans.get(element_type);
    for (field_name, field_value) in block_content {
        let include = element_plan
            .map(|p| p.include_fields.contains(field_name))
            .unwrap_or(false);
        stack.push((field_value, include));
    }
}

/// Convert a property value to text for embedding using an iterative walk.
///
/// Retained for non-plan callers. The schema-driven extraction above is
/// plan-gated and does NOT use this helper. Extracts all textual content from
/// nested Objects, Arrays, Composites, and Elements without recursion.
pub fn property_value_to_text(value: &PropertyValue) -> Option<String> {
    let mut result_parts = Vec::new();
    let mut work_stack = vec![value];

    while let Some(current) = work_stack.pop() {
        match current {
            PropertyValue::String(s) => {
                if !s.is_empty() {
                    result_parts.push(s.clone());
                }
            }
            PropertyValue::Integer(i) => result_parts.push(i.to_string()),
            PropertyValue::Float(f) => result_parts.push(f.to_string()),
            PropertyValue::Boolean(b) => result_parts.push(b.to_string()),
            PropertyValue::Date(d) => result_parts.push(d.to_string()),
            PropertyValue::Url(u) => {
                if !u.url.is_empty() {
                    result_parts.push(u.url.clone());
                }
            }
            PropertyValue::Array(arr) => {
                for item in arr.iter().rev() {
                    work_stack.push(item);
                }
            }
            PropertyValue::Object(obj) => {
                for value in obj.values() {
                    work_stack.push(value);
                }
            }
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    for value in block.content.values() {
                        work_stack.push(value);
                    }
                }
            }
            PropertyValue::Element(block) => {
                for value in block.content.values() {
                    work_stack.push(value);
                }
            }
            PropertyValue::Decimal(d) => result_parts.push(d.to_string()),
            PropertyValue::Null
            | PropertyValue::Reference(_)
            | PropertyValue::Resource(_)
            | PropertyValue::Vector(_)
            | PropertyValue::Geometry(_) => {}
        }
    }

    if result_parts.is_empty() {
        None
    } else {
        Some(result_parts.join("\n"))
    }
}

/// Hash text for detecting changes
pub fn hash_text(text: &str) -> u64 {
    // Delegates rather than reimplements: the search path recomputes this hash
    // to verify a chunk span still slices the passage that was embedded, so the
    // two must be the same function, not merely the same today.
    raisin_embeddings::hash_chunk_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::Element;
    use raisin_storage::fulltext::ElementFieldPlan;

    fn element(et: &str, fields: &[(&str, &str)]) -> PropertyValue {
        let content = fields
            .iter()
            .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
            .collect();
        PropertyValue::Element(Element {
            uuid: "u".to_string(),
            element_type: et.to_string(),
            content,
        })
    }

    /// (a) element field marked Vector is embedded, an unmarked one is NOT;
    /// (b) no "ns:Type" identity token appears; (c) top-level prop not in
    /// top_level_props is excluded.
    #[test]
    fn vector_walk_is_value_only_and_plan_gated() {
        let mut element_plans = HashMap::new();
        // Only "title" of launchpad:Hero is marked Vector; "subtitle" is not.
        element_plans.insert(
            "launchpad:Hero".to_string(),
            ElementFieldPlan {
                include_fields: ["title"].iter().map(|s| s.to_string()).collect(),
            },
        );
        let plan = NodeIndexPlan {
            node_type: "ns:Page".to_string(),
            archetype: Some("ns:ContentArchetype".to_string()),
            // Only "summary" is selected at top level.
            top_level_props: Some(vec!["summary".to_string()]),
            element_plans,
            legacy_index_all_strings: false,
        };

        let mut props = HashMap::new();
        props.insert(
            "summary".to_string(),
            PropertyValue::String("Selected summary text".to_string()),
        );
        props.insert(
            "internal".to_string(),
            PropertyValue::String("Excluded internal text".to_string()),
        );
        props.insert(
            "body".to_string(),
            element(
                "launchpad:Hero",
                &[("title", "Embedded title"), ("subtitle", "Hidden subtitle")],
            ),
        );

        let out = collect_plan_values(&plan, &props);

        // (a) marked element field IS embedded; unmarked is NOT.
        assert!(
            out.contains("Embedded title"),
            "marked element field missing"
        );
        assert!(
            !out.contains("Hidden subtitle"),
            "unmarked element field leaked"
        );
        // (a)/top-level: selected top-level prop IS embedded.
        assert!(out.contains("Selected summary text"));
        // (c) top-level prop not in top_level_props is excluded.
        assert!(
            !out.contains("Excluded internal text"),
            "non-selected top-level prop leaked"
        );
        // (b) NO shape-type identity token appears anywhere in the output.
        assert!(!out.contains("ns:Page"), "node_type identity leaked");
        assert!(!out.contains("launchpad:Hero"), "element identity leaked");
        assert!(
            !out.contains("ns:ContentArchetype"),
            "archetype identity leaked"
        );
        assert!(!out.contains("Hero"), "fuzzy identity token leaked");
    }

    #[test]
    fn unknown_element_type_contributes_no_values() {
        let plan = NodeIndexPlan {
            node_type: "ns:Page".to_string(),
            archetype: None,
            top_level_props: Some(vec![]),
            element_plans: HashMap::new(),
            legacy_index_all_strings: false,
        };
        let mut props = HashMap::new();
        props.insert(
            "body".to_string(),
            element("x:Unknown", &[("f", "secret value")]),
        );

        let out = collect_plan_values(&plan, &props);
        assert!(!out.contains("secret value"));
        assert!(!out.contains("x:Unknown"));
    }

    /// An asset's CAPTION reaches the embeddable text, including the
    /// array-valued `keywords`.
    ///
    /// The plan below is what `IndexingPolicy::properties_to_index` produces
    /// for `raisin:Asset` under `IndexType::Vector` now that
    /// `raisin_asset.yaml` marks all three caption fields `Vector` — the YAML
    /// half is pinned by
    /// `raisin_core::nodetype_init::tests::asset_caption_fields_are_vector_indexed`.
    ///
    /// `keywords` is the one worth asserting rather than assuming: it is an
    /// `Array`, and the walk carries the include flag DOWN into array items
    /// rather than deciding per item. A change that made arrays opaque would
    /// leave `description` and `alt_text` searchable and `keywords` silently
    /// not — the half-working state that is hardest to notice.
    ///
    /// `__extracted_text` is asserted ABSENT on purpose. It is deliberately not
    /// Vector-indexed here: a document body gets its own `doc` spec so a
    /// 40-page contract is not glued onto a filename and a caption, producing
    /// one vector that matches neither well.
    #[test]
    fn an_asset_caption_is_embeddable_text() {
        let plan = NodeIndexPlan {
            node_type: "raisin:Asset".to_string(),
            archetype: None,
            top_level_props: Some(vec![
                "title".to_string(),
                "description".to_string(),
                "alt_text".to_string(),
                "keywords".to_string(),
            ]),
            element_plans: HashMap::new(),
            legacy_index_all_strings: false,
        };

        let mut props = HashMap::new();
        props.insert(
            "title".to_string(),
            PropertyValue::String("IMG_4471.jpg".to_string()),
        );
        props.insert(
            "description".to_string(),
            PropertyValue::String(
                "A sailing boat hauled out on hardstanding under a tarpaulin".to_string(),
            ),
        );
        props.insert(
            "alt_text".to_string(),
            PropertyValue::String("Yacht ashore for the winter".to_string()),
        );
        props.insert(
            "keywords".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::String("marina".to_string()),
                PropertyValue::String("hardstanding".to_string()),
            ]),
        );
        // Vaulted by the extraction job, NOT part of the default spec.
        props.insert(
            "__extracted_text".to_string(),
            PropertyValue::String("PAGE ONE OF THE MANUAL".to_string()),
        );
        // Engine bookkeeping: never embeddable.
        props.insert(
            "__extract_fingerprint".to_string(),
            PropertyValue::String("sha256:deadbeef".to_string()),
        );

        let out = collect_plan_values(&plan, &props);

        assert!(
            out.contains("hauled out on hardstanding"),
            "`description` must be embeddable: {out}"
        );
        assert!(
            out.contains("Yacht ashore for the winter"),
            "`alt_text` must be embeddable: {out}"
        );
        assert!(
            out.contains("marina") && out.contains("hardstanding"),
            "EVERY `keywords` item must be embeddable, not just the first: {out}"
        );
        assert!(
            !out.contains("PAGE ONE OF THE MANUAL"),
            "`__extracted_text` rides its own `doc` spec and must not be glued \
             onto the caption vector: {out}"
        );
        assert!(
            !out.contains("deadbeef"),
            "engine bookkeeping must never reach an embedding: {out}"
        );
    }

    #[test]
    fn legacy_plan_embeds_all_top_level_strings() {
        let plan = NodeIndexPlan {
            node_type: "ns:Untyped".to_string(),
            archetype: None,
            top_level_props: None,
            element_plans: HashMap::new(),
            legacy_index_all_strings: true,
        };
        let mut props = HashMap::new();
        props.insert("a".to_string(), PropertyValue::String("alpha".to_string()));
        props.insert("b".to_string(), PropertyValue::String("beta".to_string()));

        let out = collect_plan_values(&plan, &props);
        assert!(out.contains("alpha"));
        assert!(out.contains("beta"));
    }
}
