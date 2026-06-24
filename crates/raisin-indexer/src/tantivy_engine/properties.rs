// SPDX-License-Identifier: BSL-1.1

//! Shape-driven property extraction and flattening for full-text indexing.
//!
//! Indexing is driven by a precomputed [`NodeIndexPlan`]: shape identities
//! (node_type / archetype / every nested element_type) are always emitted so a
//! node is findable by the shapes it carries, while field *values* are indexed
//! only when the field's `index` config includes `Fulltext`.

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::fulltext::NodeIndexPlan;
use std::collections::{HashMap, HashSet};

/// Result of flattening a node's properties for one document.
pub(crate) struct FlattenOutput {
    /// Free-text content (field values + identity tokens) for the `content` field.
    pub content: String,
    /// Distinct shape-type identities for the multi-valued `shape_types` field.
    pub shape_types: Vec<String>,
}

/// Flattens node properties into searchable text + shape identities per the plan.
pub(crate) fn flatten_properties(
    plan: &NodeIndexPlan,
    properties: &HashMap<String, PropertyValue>,
) -> FlattenOutput {
    let mut content: Vec<String> = Vec::new();
    let mut shape_types: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1. Shape identities that are always searchable.
    push_identity(&plan.node_type, &mut content, &mut shape_types, &mut seen);
    if let Some(archetype) = &plan.archetype {
        push_identity(archetype, &mut content, &mut shape_types, &mut seen);
    }

    // 2. Per-property values + nested element identities.
    let selected: Option<HashSet<&str>> = plan
        .top_level_props
        .as_ref()
        .map(|p| p.iter().map(|s| s.as_str()).collect());

    for (name, value) in properties {
        // Decide whether this top-level property's value is indexed by value.
        let index_value = match &selected {
            Some(set) => set.contains(name.as_str()),
            None => plan.legacy_index_all_strings && matches!(value, PropertyValue::String(_)),
        };
        if index_value {
            // Field name token preserves "search by field name" behavior.
            content.push(format!("{}:", name));
        }
        walk(
            value,
            index_value,
            plan,
            &mut content,
            &mut shape_types,
            &mut seen,
        );
    }

    FlattenOutput {
        content: content.join(" "),
        shape_types,
    }
}

/// Iterative, overflow-safe walk emitting nested element identities (always) and
/// scalar values (gated). Element/Composite sub-fields are gated by the element
/// type's own `include_fields`, independent of the incoming `index` flag, so a
/// field is indexed exactly when its own config says so.
fn walk(
    value: &PropertyValue,
    index_value: bool,
    plan: &NodeIndexPlan,
    content: &mut Vec<String>,
    shape_types: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let mut stack: Vec<(&PropertyValue, bool)> = vec![(value, index_value)];

    while let Some((current, idx)) = stack.pop() {
        match current {
            PropertyValue::String(s) => {
                if idx && !s.is_empty() {
                    content.push(s.clone());
                }
            }
            PropertyValue::Integer(n) => {
                if idx {
                    content.push(n.to_string());
                }
            }
            PropertyValue::Float(f) => {
                if idx {
                    content.push(f.to_string());
                }
            }
            PropertyValue::Boolean(b) => {
                if idx {
                    content.push(b.to_string());
                }
            }
            PropertyValue::Date(d) => {
                if idx {
                    content.push(d.to_string());
                }
            }
            PropertyValue::Decimal(dec) => {
                if idx {
                    content.push(dec.to_string());
                }
            }
            PropertyValue::Url(u) => {
                if idx {
                    content.push(u.url.clone());
                    for extra in [&u.label, &u.title, &u.description].into_iter().flatten() {
                        if !extra.is_empty() {
                            content.push(extra.clone());
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
                push_identity(&block.element_type, content, shape_types, seen);
                push_element_content(&block.element_type, &block.content, plan, &mut stack);
            }
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    push_identity(&block.element_type, content, shape_types, seen);
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
/// field by the element type's `include_fields` plan. An unresolved/unknown
/// element type indexes none of its values (identity-only default).
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

/// Emits a shape-type identity once: as an exact `shape_types` term, and as
/// fuzzy `content` tokens (full, local-after-`:`, and `:`-separated forms) so it
/// is discoverable both by the exact filter and by plain full-text search.
fn push_identity(
    name: &str,
    content: &mut Vec<String>,
    shape_types: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if name.is_empty() || !seen.insert(name.to_string()) {
        return;
    }
    shape_types.push(name.to_string());
    content.push(name.to_string());
    if let Some((_, local)) = name.rsplit_once(':') {
        if !local.is_empty() {
            content.push(local.to_string());
        }
    }
    if name.contains(':') {
        content.push(name.replace(':', " "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::value::{Composite, Element};
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

    fn plan_with_elements(node_type: &str, elements: &[(&str, &[&str])]) -> NodeIndexPlan {
        let element_plans = elements
            .iter()
            .map(|(et, fields)| {
                (
                    et.to_string(),
                    ElementFieldPlan {
                        include_fields: fields.iter().map(|s| s.to_string()).collect(),
                    },
                )
            })
            .collect();
        NodeIndexPlan {
            node_type: node_type.to_string(),
            archetype: None,
            top_level_props: Some(vec![]),
            element_plans,
            legacy_index_all_strings: false,
        }
    }

    #[test]
    fn nested_element_identity_is_indexed() {
        // Regression: a nested element's element_type must surface in shape_types
        // and as fuzzy content tokens, even when none of its fields are indexed.
        let plan = plan_with_elements("ns:Page", &[("launchpad:Hero", &[])]);
        let mut props = HashMap::new();
        props.insert(
            "body".to_string(),
            element("launchpad:Hero", &[("title", "Hi")]),
        );

        let out = flatten_properties(&plan, &props);

        assert!(out.shape_types.contains(&"launchpad:Hero".to_string()));
        assert!(out.shape_types.contains(&"ns:Page".to_string()));
        // Fuzzy tokens: full, local-after-colon, and colon-replaced forms.
        assert!(out.content.contains("launchpad:Hero"));
        assert!(out.content.contains("Hero"));
        // Field value not configured for fulltext -> not indexed.
        assert!(!out.content.contains("Hi"));
    }

    #[test]
    fn element_field_gating_respects_include_fields() {
        let plan = plan_with_elements("ns:Page", &[("launchpad:Hero", &["title"])]);
        let mut props = HashMap::new();
        props.insert(
            "body".to_string(),
            element(
                "launchpad:Hero",
                &[("title", "Indexed"), ("subtitle", "Hidden")],
            ),
        );

        let out = flatten_properties(&plan, &props);

        assert!(out.content.contains("Indexed"));
        assert!(!out.content.contains("Hidden"));
    }

    #[test]
    fn composite_blocks_each_emit_identity() {
        let plan = plan_with_elements("ns:Page", &[("a:One", &[]), ("a:Two", &[])]);
        let composite = PropertyValue::Composite(Composite {
            uuid: "c".to_string(),
            items: vec![
                if let PropertyValue::Element(e) = element("a:One", &[]) {
                    e
                } else {
                    unreachable!()
                },
                if let PropertyValue::Element(e) = element("a:Two", &[]) {
                    e
                } else {
                    unreachable!()
                },
            ],
        });
        let mut props = HashMap::new();
        props.insert("blocks".to_string(), composite);

        let out = flatten_properties(&plan, &props);

        assert!(out.shape_types.contains(&"a:One".to_string()));
        assert!(out.shape_types.contains(&"a:Two".to_string()));
    }

    #[test]
    fn unknown_element_type_is_identity_only() {
        // No plan entry for the element type -> identity emitted, values skipped.
        let plan = plan_with_elements("ns:Page", &[]);
        let mut props = HashMap::new();
        props.insert("body".to_string(), element("x:Unknown", &[("f", "secret")]));

        let out = flatten_properties(&plan, &props);

        assert!(out.shape_types.contains(&"x:Unknown".to_string()));
        assert!(!out.content.contains("secret"));
    }

    #[test]
    fn typed_node_indexes_only_selected_top_level_props() {
        let mut plan = plan_with_elements("ns:Page", &[]);
        plan.top_level_props = Some(vec!["title".to_string()]);
        let mut props = HashMap::new();
        props.insert(
            "title".to_string(),
            PropertyValue::String("Keep".to_string()),
        );
        props.insert(
            "internal".to_string(),
            PropertyValue::String("Drop".to_string()),
        );

        let out = flatten_properties(&plan, &props);

        assert!(out.content.contains("Keep"));
        assert!(!out.content.contains("Drop"));
    }

    #[test]
    fn legacy_plan_indexes_all_top_level_strings() {
        let plan = NodeIndexPlan {
            node_type: "ns:Page".to_string(),
            archetype: None,
            top_level_props: None,
            element_plans: HashMap::new(),
            legacy_index_all_strings: true,
        };
        let mut props = HashMap::new();
        props.insert("a".to_string(), PropertyValue::String("alpha".to_string()));
        props.insert("b".to_string(), PropertyValue::String("beta".to_string()));

        let out = flatten_properties(&plan, &props);

        assert!(out.content.contains("alpha"));
        assert!(out.content.contains("beta"));
    }

    #[test]
    fn deeply_nested_elements_do_not_overflow() {
        // Build a deep chain of elements nested via content. The flatten walk is
        // iterative (explicit stack), so it handles depth in O(1) stack frames;
        // depth is kept modest here only because the *fixture's* recursive Drop
        // would otherwise overflow at teardown.
        let mut inner = element("deep:Leaf", &[]);
        for _ in 0..1000 {
            let mut content = HashMap::new();
            content.insert("child".to_string(), inner);
            inner = PropertyValue::Element(Element {
                uuid: "u".to_string(),
                element_type: "deep:Node".to_string(),
                content,
            });
        }
        let plan = plan_with_elements("ns:Page", &[]);
        let mut props = HashMap::new();
        props.insert("root".to_string(), inner);

        let out = flatten_properties(&plan, &props);
        // Both distinct nested types discovered without stack overflow.
        assert!(out.shape_types.contains(&"deep:Node".to_string()));
        assert!(out.shape_types.contains(&"deep:Leaf".to_string()));
    }
}
