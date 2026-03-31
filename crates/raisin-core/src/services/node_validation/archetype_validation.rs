//! Archetype validation.
//!
//! Validates node archetype associations, including base_node_type constraints,
//! field validation against resolved archetype schemas, and strict mode checks.

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_storage::Storage;
use raisin_validation::field_name;
use std::collections::{HashMap, HashSet};

use crate::services::archetype_resolver::ResolvedArchetype;
use crate::services::element_type_resolver::ResolvedElementType;

use super::core::NodeValidator;

impl<S: Storage> NodeValidator<S> {
    /// Validate archetype association and field constraints
    pub(super) async fn validate_archetype(
        &self,
        node: &Node,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        let Some(archetype_name) = node.archetype.as_deref() else {
            return Ok(());
        };

        // Resolve archetype with full inheritance chain
        let resolved = self
            .archetype_resolver
            .resolve(archetype_name)
            .await
            .map_err(|e| {
                Error::Validation(format!(
                    "Failed to resolve archetype '{}' for node '{}': {}",
                    archetype_name, node.id, e
                ))
            })?;

        // Check base_node_type constraint
        if let Some(base) = &resolved.archetype.base_node_type {
            if base != &node.node_type {
                return Err(Error::Validation(format!(
                    "Archetype '{}' is only valid for node type '{}', but node '{}' uses '{}'",
                    archetype_name, base, node.id, node.node_type
                )));
            }
        }

        // Validate against resolved fields (includes inherited fields from parent archetypes)
        if !resolved.resolved_fields.is_empty() {
            self.validate_fields_against_schema(
                &node.properties,
                &resolved.resolved_fields,
                &format!("archetype '{}'", archetype_name),
                cache,
            )
            .await?;
        }

        // Check strict mode for archetype (no undefined properties allowed)
        if resolved.resolved_strict {
            self.check_archetype_strict_mode(node, &resolved)?;
        }

        Ok(())
    }

    /// Check that no undefined properties exist for archetype (strict mode)
    fn check_archetype_strict_mode(&self, node: &Node, resolved: &ResolvedArchetype) -> Result<()> {
        // Build set of allowed field names from resolved archetype
        let allowed_fields: HashSet<&str> =
            resolved.resolved_fields.iter().map(field_name).collect();

        // Check each node property against allowed archetype fields
        for key in node.properties.keys() {
            if !allowed_fields.contains(key.as_str()) {
                return Err(Error::Validation(format!(
                    "Undefined property '{}' in strict archetype '{}'",
                    key, resolved.archetype.name
                )));
            }
        }

        Ok(())
    }
}
