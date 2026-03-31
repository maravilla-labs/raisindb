//! Recursive NodeType resolution with inheritance, mixins, and circular dependency detection.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{IndexType, PropertyValueSchema};
use raisin_storage::{scope::BranchScope, NodeTypeRepository, Storage};
use std::collections::{HashMap, HashSet};

use super::{NodeTypeResolver, ResolvedNodeType, MAX_INHERITANCE_DEPTH};

impl<S: Storage> NodeTypeResolver<S> {
    /// Recursive resolution with circular dependency detection
    pub(super) fn resolve_recursive<'a>(
        &'a self,
        node_type_name: &'a str,
        visited: &'a mut HashSet<String>,
        chain: &'a mut Vec<String>,
        pins: Option<&'a HashMap<String, Option<HLC>>>,
        revision_cache: &'a mut HashMap<String, HLC>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ResolvedNodeType>> + Send + 'a>>
    {
        Box::pin(async move {
            if visited.contains(node_type_name) {
                return Err(Error::Validation(format!(
                    "Circular dependency detected in NodeType inheritance: {} -> {}",
                    chain.join(" -> "),
                    node_type_name
                )));
            }

            if chain.len() >= MAX_INHERITANCE_DEPTH {
                return Err(Error::Validation(format!(
                    "Maximum inheritance depth ({}) exceeded. Chain: {}",
                    MAX_INHERITANCE_DEPTH,
                    chain.join(" -> ")
                )));
            }

            visited.insert(node_type_name.to_string());
            chain.push(node_type_name.to_string());

            let max_revision = if let Some(pins_map) = pins {
                match pins_map.get(node_type_name) {
                    Some(Some(pin_value)) => {
                        if let Some(cached) = revision_cache.get(node_type_name) {
                            Some(*cached)
                        } else {
                            let revision = self
                                .resolve_revision_for_pin(node_type_name, *pin_value)
                                .await?;
                            revision_cache.insert(node_type_name.to_string(), revision);
                            Some(revision)
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };

            let repo = self.storage.node_types();
            let node_type = repo
                .get(
                    BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                    node_type_name,
                    max_revision.as_ref(),
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("NodeType not found: {}", node_type_name))
                })?;

            let mut resolved_allowed_children = Vec::new();
            let mut property_map: HashMap<String, PropertyValueSchema> = HashMap::new();

            let mut resolved_indexable = true;
            let mut resolved_index_types: Vec<IndexType> =
                vec![IndexType::Fulltext, IndexType::Vector, IndexType::Property];

            // 1. Resolve parent (extends) first
            if let Some(ref extends) = node_type.extends {
                let parent_resolved = self
                    .resolve_recursive(extends, visited, chain, pins, revision_cache)
                    .await?;

                for prop in parent_resolved.resolved_properties {
                    if let Some(ref name) = prop.name {
                        property_map.insert(name.clone(), prop);
                    }
                }
                resolved_allowed_children.extend(parent_resolved.resolved_allowed_children);
                resolved_indexable = parent_resolved.resolved_indexable;
                resolved_index_types = parent_resolved.resolved_index_types;
            }

            // 2. Apply mixins in order
            if !node_type.mixins.is_empty() {
                for mixin_name in &node_type.mixins {
                    let mut mixin_visited = HashSet::new();
                    let mut mixin_chain = Vec::new();
                    let mixin_resolved = self
                        .resolve_recursive(
                            mixin_name,
                            &mut mixin_visited,
                            &mut mixin_chain,
                            pins,
                            revision_cache,
                        )
                        .await?;

                    for prop in mixin_resolved.resolved_properties {
                        if let Some(ref name) = prop.name {
                            property_map.insert(name.clone(), prop);
                        }
                    }

                    for child in mixin_resolved.resolved_allowed_children {
                        if !resolved_allowed_children.contains(&child) {
                            resolved_allowed_children.push(child);
                        }
                    }

                    if !mixin_resolved.resolved_indexable {
                        resolved_indexable = false;
                    }
                    resolved_index_types
                        .retain(|t| mixin_resolved.resolved_index_types.contains(t));
                }
            }

            // 3. Apply current NodeType's own properties
            if let Some(ref properties) = node_type.properties {
                for prop in properties {
                    if let Some(ref name) = prop.name {
                        property_map.insert(name.clone(), prop.clone());
                    }
                }
            }

            // 4. Apply overrides
            if let Some(ref overrides) = node_type.overrides {
                for (prop_name, override_value) in overrides {
                    if let Some(prop_schema) = property_map.get_mut(prop_name) {
                        prop_schema.default = Some(override_value.clone());
                    }
                }
            }

            // 5. Merge allowed_children
            if !node_type.allowed_children.is_empty() {
                for child in &node_type.allowed_children {
                    if !resolved_allowed_children.contains(child) {
                        resolved_allowed_children.push(child.clone());
                    }
                }
            }

            // 6. Apply current NodeType's own indexing settings
            if let Some(indexable) = node_type.indexable {
                resolved_indexable = indexable;
            }
            if let Some(ref index_types) = node_type.index_types {
                resolved_index_types = index_types.clone();
            }

            let mut resolved_properties: Vec<PropertyValueSchema> =
                property_map.into_values().collect();
            resolved_properties.sort_by(|a, b| a.name.cmp(&b.name));

            Ok(ResolvedNodeType {
                node_type,
                resolved_properties,
                resolved_allowed_children,
                resolved_indexable,
                resolved_index_types,
                inheritance_chain: chain.clone(),
            })
        })
    }

    pub(super) async fn resolve_revision_for_pin(
        &self,
        node_type_name: &str,
        pin_hlc: HLC,
    ) -> Result<HLC> {
        let repo = self.storage.node_types();
        let node_type = repo
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                node_type_name,
                Some(&pin_hlc),
            )
            .await?;

        if node_type.is_none() {
            return Err(Error::Validation(format!(
                "Pinned revision {:?} for NodeType '{}' does not exist",
                pin_hlc, node_type_name
            )));
        }

        Ok(pin_hlc)
    }
}
