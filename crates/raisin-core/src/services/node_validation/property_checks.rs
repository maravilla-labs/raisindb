//! Property validation checks.
//!
//! Validates required properties, strict mode constraints, and unique property
//! constraints against NodeType schemas.

use raisin_error::{Error, Result};
use raisin_indexer::IndexQuery;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::collections::HashMap;

use crate::services::node_type_resolver::ResolvedNodeType;

use super::core::NodeValidator;

impl<S: Storage> NodeValidator<S> {
    /// Check that all required properties are present
    pub(super) fn check_required_properties(
        &self,
        node: &Node,
        resolved: &ResolvedNodeType,
    ) -> Result<()> {
        for schema in &resolved.resolved_properties {
            let is_required = schema.required.unwrap_or(false);
            if is_required {
                let prop_name = schema
                    .name
                    .as_ref()
                    .ok_or_else(|| Error::Validation("Property schema has no name".to_string()))?;

                if !node.properties.contains_key(prop_name) {
                    return Err(Error::Validation(format!(
                        "Missing required property '{}' for NodeType '{}'",
                        prop_name, node.node_type
                    )));
                }
            }
        }
        Ok(())
    }

    /// Check that no undefined properties exist (strict mode)
    pub(super) fn check_strict_mode(&self, node: &Node, resolved: &ResolvedNodeType) -> Result<()> {
        // Build set of allowed property names
        let allowed_properties: HashMap<&str, ()> = resolved
            .resolved_properties
            .iter()
            .filter_map(|schema| schema.name.as_deref().map(|n| (n, ())))
            .collect();

        // Check each node property is defined in schema
        for key in node.properties.keys() {
            if !allowed_properties.contains_key(key.as_str()) {
                return Err(Error::Validation(format!(
                    "Undefined property '{}' in strict mode for NodeType '{}'",
                    key, node.node_type
                )));
            }
        }

        Ok(())
    }

    /// Check unique property constraints
    pub(super) async fn check_unique_properties(
        &self,
        workspace: &str,
        node: &Node,
        resolved: &ResolvedNodeType,
    ) -> Result<()> {
        let node_repo = self.storage.nodes();

        for schema in &resolved.resolved_properties {
            if schema.unique.unwrap_or(false) {
                let prop_name = match &schema.name {
                    Some(n) => n,
                    None => continue,
                };

                // Check if this property has a value in the node
                if let Some(property_value) = node.properties.get(prop_name) {
                    // Query for conflicting nodes
                    if let Some(conflicting) = self
                        .find_conflicting_node(
                            workspace,
                            &node.id,
                            prop_name,
                            property_value,
                            node_repo,
                        )
                        .await?
                    {
                        return Err(Error::Validation(format!(
                            "Property '{}' must be unique, but node '{}' (id: '{}') has the same value",
                            prop_name, conflicting.name, conflicting.id
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Find a node with conflicting unique property value
    async fn find_conflicting_node(
        &self,
        workspace: &str,
        current_node_id: &str,
        prop_name: &str,
        prop_value: &PropertyValue,
        node_repo: &S::Nodes,
    ) -> Result<Option<Node>> {
        // Try using index first if available (O(1) lookup)
        if let Some(ref index_mgr) = self.index_manager {
            // Use repository-scoped workspace key expected by PropertyIndexPlugin
            // Format: "{tenant}/{repo}/{branch}" (branchless storage, branch at service level)
            let workspace_key = format!("{}/{}/{}", self.tenant_id, self.repo_id, self.branch);
            let query = IndexQuery::FindByProperty {
                workspace: workspace_key,
                property_name: prop_name.to_string(),
                property_value: Box::new(prop_value.clone()),
            };

            // Query the property_unique index
            if let Ok(node_ids) = index_mgr.query("property_unique", query).await {
                // Check if any of the found nodes is different from current node
                for node_id in node_ids {
                    if node_id != current_node_id {
                        // Load the node to return it
                        let scope = StorageScope::new(
                            &self.tenant_id,
                            &self.repo_id,
                            &self.branch,
                            workspace,
                        );
                        if let Some(node) = node_repo.get(scope, &node_id, None).await? {
                            return Ok(Some(node));
                        }
                    }
                }
                return Ok(None);
            }
        }

        // Fallback to O(n) scan if index not available or query failed
        let scope = StorageScope::new(&self.tenant_id, &self.repo_id, &self.branch, workspace);
        let all_nodes = node_repo
            .list_all(scope, raisin_storage::ListOptions::for_api())
            .await?;

        for node in all_nodes {
            // Skip the current node
            if node.id == current_node_id {
                continue;
            }

            // Check if this node has the same property value
            if let Some(other_value) = node.properties.get(prop_name) {
                if Self::property_values_equal(prop_value, other_value) {
                    return Ok(Some(node));
                }
            }
        }

        Ok(None)
    }

    /// Compare two property values for equality
    pub(super) fn property_values_equal(a: &PropertyValue, b: &PropertyValue) -> bool {
        match (a, b) {
            (PropertyValue::String(s1), PropertyValue::String(s2)) => s1 == s2,
            (PropertyValue::Integer(n1), PropertyValue::Integer(n2)) => n1 == n2,
            (PropertyValue::Float(n1), PropertyValue::Float(n2)) => n1 == n2,
            (PropertyValue::Boolean(b1), PropertyValue::Boolean(b2)) => b1 == b2,
            (PropertyValue::Date(d1), PropertyValue::Date(d2)) => d1 == d2,
            (PropertyValue::Url(u1), PropertyValue::Url(u2)) => u1.url == u2.url,
            (PropertyValue::Reference(r1), PropertyValue::Reference(r2)) => r1.id == r2.id,
            (PropertyValue::Array(a1), PropertyValue::Array(a2)) => a1 == a2,
            (PropertyValue::Object(o1), PropertyValue::Object(o2)) => o1 == o2,
            (PropertyValue::Element(b1), PropertyValue::Element(b2)) => b1.uuid == b2.uuid,
            (PropertyValue::Composite(bc1), PropertyValue::Composite(bc2)) => bc1.uuid == bc2.uuid,
            (PropertyValue::Resource(r1), PropertyValue::Resource(r2)) => r1.uuid == r2.uuid,
            _ => false,
        }
    }
}
